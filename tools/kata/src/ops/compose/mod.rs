//! Composing a release line from carried and selected payloads.

use alloc::borrow::ToOwned;
use std::path::{Path, PathBuf};

use koci::registry;

use crate::error::{KataError, Result};
use crate::ops::reference;
use crate::ops::verify;
use crate::repository;
use crate::schema::documents::Document;
use crate::schema::kinds::Kind;
use crate::schema::parse::validate_release;

pub(crate) mod auto;
pub(crate) mod lines;
pub(crate) mod merge;
pub mod plan;

use merge::Bases;
use plan::{Auto, Selections};

/// How the composition result is handled.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    /// Print the resolved plan and exit without writing.
    Print,
    /// Write the composed documents to the output root.
    Write,
}

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
    /// Output docs root (scratch for local runs, the repo for CI).
    pub dir: PathBuf,
    /// Derivation root (catalog repo checkout).
    pub from: Option<PathBuf>,
    /// Lineage parent line; defaults to the newest line of `from`.
    pub from_line: Option<String>,
    /// Release line being composed.
    pub release: String,
    /// Payload tag selections.
    pub selections: Selections,
    /// Ignore carried and scratch pins.
    pub fresh: bool,
    /// Replace an already-published line (dev scratch registries only).
    pub force: bool,
    /// Compose from a line that is not the newest one.
    pub allow_outdated_from: bool,
    /// Tag policy for entries without an explicit selection.
    pub policy: Policy,
    /// Print the plan instead of writing.
    pub mode: Mode,
    /// Registry prefix for resolution and publication checks.
    pub registry: String,
}

/// Compose the release line; returns the written document paths.
///
/// # Errors
///
/// Returns an error when the lineage or publication gates refuse the
/// composition, a selection or carried pin cannot be resolved, or a document
/// fails verification. Nothing is written when any gate or verification
/// fails.
pub fn run(input: &Input) -> Result<Vec<String>> {
    validate_release(&input.release)?;

    let carried = carried_documents(
        input.from.as_ref(),
        input.from_line.as_deref(),
        input.allow_outdated_from,
    )?;
    let carried_bases = carried.as_ref().map(carried_bases).transpose()?;
    let output = output_documents(&input.dir, &input.release);
    let (mut bases, origins) =
        merge::assemble(&output, carried_bases.as_ref(), input.fresh, &input.release);
    let resolve = |repository: &str, tag: &str| {
        let line_reference = reference(&input.registry, repository, tag);
        koci::registry::manifest_digest(&line_reference)
            .map_err(|error| KataError::Registry(error.to_string()))
    };
    let policy = (input.policy == Policy::Auto)
        .then_some(|repository: &str| auto::newest_tag(&input.registry, repository));
    let plan = plan::build(
        &mut bases,
        &origins,
        &input.selections,
        &input.release,
        &resolve,
        policy.as_ref().map(coerce),
    )?;

    if input.mode == Mode::Print {
        plan::print(&plan)?;

        return Ok(Vec::new());
    }

    gate_publication(
        &plan.documents,
        &input.release,
        &input.registry,
        input.force,
    )?;
    for document in &plan.documents {
        verify::verify_document(document, &input.registry)?;
    }
    let written = write_documents(&plan.documents, &input.dir, &input.release)?;
    eprintln!(
        "Composed line {} ({} entries)",
        input.release,
        plan.entries.len()
    );

    Ok(written)
}

struct Carried {
    root: PathBuf,
    line: String,
}

fn coerce<F: Fn(&str) -> Result<String>>(policy: &F) -> &Auto<'_> {
    policy
}

fn carried_documents(
    from: Option<&PathBuf>,
    from_line: Option<&str>,
    allow_outdated_from: bool,
) -> Result<Option<Carried>> {
    let Some(root) = from else {
        return Ok(None);
    };

    let lines = lines::scan(root);
    let line = match from_line {
        Some(explicit) => {
            check_lineage(root, explicit, &lines, allow_outdated_from)?;
            explicit.to_owned()
        }
        None => lines::newest(root).ok_or_else(|| {
            KataError::Lineage(format!(
                "the derivation root {} carries no lines",
                root.display()
            ))
        })?,
    };
    if !repository::document_path(Kind::Core, root, &line).exists() {
        return Err(KataError::Lineage(format!(
            "the derivation root {} carries no core document for {line}",
            root.display()
        )));
    }

    Ok(Some(Carried {
        root: root.clone(),
        line,
    }))
}

fn check_lineage(
    root: &Path,
    explicit: &str,
    lines: &[String],
    allow_outdated: bool,
) -> Result<()> {
    let Some(newest) = lines.last() else {
        return Ok(());
    };
    if lines::compare(explicit, newest) != std::cmp::Ordering::Less || allow_outdated {
        return Ok(());
    }

    let from_set = lines::identities(root, explicit)?;
    let newest_set = lines::identities(root, newest)?;
    let missing: Vec<String> = newest_set
        .difference(&from_set)
        .map(ToOwned::to_owned)
        .collect();

    Err(KataError::Lineage(format!(
        "--from-line {explicit} is not the newest line ({newest}) and lacks {} — \
         pass --allow-outdated-from to proceed",
        missing.join(", ")
    )))
}

fn output_documents(dir: &Path, release: &str) -> Bases {
    let core = optional(Kind::Core, dir, release).and_then(|document| match document {
        Document::Core(core) => Some(core),
        Document::Overlays(_) | Document::Extensions(_) => None,
    });
    let overlays = optional(Kind::Overlays, dir, release).and_then(|document| match document {
        Document::Overlays(overlays) => Some(overlays),
        Document::Core(_) | Document::Extensions(_) => None,
    });
    let extensions = optional(Kind::Extensions, dir, release).and_then(|document| match document {
        Document::Extensions(extensions) => Some(extensions),
        Document::Core(_) | Document::Overlays(_) => None,
    });

    Bases {
        core,
        overlays,
        extensions,
    }
}

fn carried_bases(carried: &Carried) -> Result<Bases> {
    let root = &carried.root;
    let line = &carried.line;

    let core = match optional(Kind::Core, root, line) {
        Some(Document::Core(core)) => Some(core),
        Some(_) => {
            return Err(KataError::Document(format!(
                "core/{line}.toml is not a core document"
            )));
        }
        None => {
            return Err(KataError::Document(format!(
                "the derivation root carries no core document for {line}"
            )));
        }
    };
    let overlays = match optional(Kind::Overlays, root, line) {
        Some(Document::Overlays(overlays)) => Some(overlays),
        Some(_) => {
            return Err(KataError::Document(format!(
                "overlays/{line}.toml is not an overlays document"
            )));
        }
        None => None,
    };
    let extensions = match optional(Kind::Extensions, root, line) {
        Some(Document::Extensions(extensions)) => Some(extensions),
        Some(_) => {
            return Err(KataError::Document(format!(
                "extensions/{line}.toml is not an extensions document"
            )));
        }
        None => None,
    };

    Ok(Bases {
        core,
        overlays,
        extensions,
    })
}

fn optional(kind: Kind, root: &Path, release: &str) -> Option<Document> {
    if repository::document_path(kind, root, release).exists() {
        repository::load(kind, root, release).ok()
    } else {
        None
    }
}

fn gate_publication(
    documents: &[Document],
    release: &str,
    registry: &str,
    force: bool,
) -> Result<()> {
    if force {
        return Ok(());
    }

    for document in documents {
        let line_reference = reference(registry, document.kind().repository(), release);
        if registry::manifest_exists(&line_reference)
            .map_err(|error| KataError::Registry(error.to_string()))?
        {
            return Err(KataError::Gate(format!(
                "{line_reference} is already published; lines are append-only — \
                 compose a new line, or pass --force (dev scratch registries only)"
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
