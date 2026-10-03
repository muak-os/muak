//! Composing a release line from carried and selected payloads.

use std::path::{Path, PathBuf};

use koci::registry;

use crate::error::{KataError, Result};
use crate::ops::reference;
use crate::ops::verify;
use crate::repository;
use crate::schema::documents;
use crate::schema::documents::Document;
use crate::schema::kinds::Kind;
use crate::schema::parse::validate_release;
use crate::version;

pub(crate) mod apply;
pub(crate) mod auto;
pub(crate) mod bases;
pub(crate) mod entries;
pub(crate) mod introductions;
pub(crate) mod lines;
pub(crate) mod merge;
pub(crate) mod pins;
pub mod plan;
pub(crate) mod selection;

use bases::{output_documents, parent_bases, parent_line};
use plan::{Auto, Selections};

/// Tag policy for entries without an explicit selection.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Policy {
    /// Carry unselected entries verbatim.
    Carried,
    /// Bump unselected entries to their newest version tag.
    Auto,
}

/// One `kata compose` request.
pub struct Input {
    /// Docs root holding the release lines.
    pub dir: PathBuf,
    /// Lineage parent line; defaults to the newest line other than `release`.
    pub from_line: Option<String>,
    /// Release line being composed.
    pub release: String,
    /// Payload tag selections.
    pub selections: Selections,
    /// Replace an already-published line (dev scratch registries only).
    pub force: bool,
    /// Tag policy for entries without an explicit selection.
    pub policy: Policy,
}

/// Compose the release line; returns the written document paths.
///
/// # Errors
///
/// Returns an error when the lineage or publication gates refuse the
/// composition, a selection or carried pin cannot be resolved, or a document
/// fails verification. Nothing is written when any gate or verification fails.
pub fn run(input: &Input) -> Result<Vec<String>> {
    validate_release(&input.release)?;
    version::ensure_line(
        &input.release,
        repository::document_path(Kind::Core, &input.dir, &input.release).exists(),
        input.force,
    )?;

    let parent = parent_line(&input.dir, &input.release, input.from_line.as_deref())?;
    let parent_docs = parent
        .as_ref()
        .map(|line| parent_bases(&input.dir, line))
        .transpose()?;
    let output = output_documents(&input.dir, &input.release)?;
    let (mut bases, origins) = merge::assemble(&output, parent_docs.as_ref(), &input.release);
    let resolve = |repository: &str, tag: &str| {
        let line_reference = reference(repository, tag);
        koci::registry::manifest_digest(&line_reference)
            .map_err(|error| KataError::Registry(error.to_string()))
    };
    let policy =
        (input.policy == Policy::Auto).then_some(|repository: &str| auto::newest_tag(repository));
    let plan = plan::build(
        &mut bases,
        &origins,
        &input.selections,
        &input.release,
        &resolve,
        policy.as_ref().map(coerce),
    )?;

    gate_publication(&plan.documents, &input.release, input.force)?;
    for document in &plan.documents {
        if let Document::Core(ref core) = *document {
            documents::ensure_resolvable(core)?;
        }
        verify::verify_document(document)?;
    }
    let written = write_documents(&plan.documents, &input.dir, &input.release)?;
    eprintln!(
        "Composed line {} ({} entries)",
        input.release,
        plan.entries.len()
    );

    Ok(written)
}

fn coerce<F: Fn(&str) -> Result<String>>(policy: &F) -> &Auto<'_> {
    policy
}

fn gate_publication(documents: &[Document], release: &str, force: bool) -> Result<()> {
    if force {
        return Ok(());
    }

    for document in documents {
        let line_reference = reference(document.kind().dir(), release);
        if registry::manifest_exists(&line_reference)
            .map_err(|error| KataError::Registry(error.to_string()))?
        {
            return Err(KataError::Gate(format!(
                "{line_reference} is already published; lines are append-only. \
                 Compose a new line, or pass --force (dev scratch registries only)"
            )));
        }
    }

    Ok(())
}

fn write_documents(documents: &[Document], dir: &Path, release: &str) -> Result<Vec<String>> {
    let mut written = Vec::new();
    for document in documents {
        written.push(
            repository::write(document, dir, release)?
                .display()
                .to_string(),
        );
    }

    Ok(written)
}
