//! Public artifact build API.

use std::io::Write;
use std::path::PathBuf;
use std::sync::OnceLock;

use koci::pull::cache;
use sbolt::keys::SigningPair;
use serde::{Deserialize, Serialize};

use crate::artifact::Artifact;
use crate::error::{Result, WizardError};
use crate::profile::Profile;
use crate::resolve::{self, Sources};
use crate::source::{extension, installer, overlay::Overlay};

pub(crate) mod archive;
pub(crate) mod artifacts;
pub(crate) mod media;
pub(crate) mod uki;

/// PE section metadata needed for TPM PCR#11 prediction.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SectionInfo {
    /// PE section name (e.g. ".linux", ".initrd", ".cmdline").
    pub name: String,
    /// File offset of the section data within the PE image.
    pub file_offset: usize,
    /// Size of the section data in bytes.
    pub size: usize,
    /// SHA-256 hash of the section data.
    pub hash: [u8; 32],
}

/// Artifact build metadata.
pub struct Metadata {
    /// PE section descriptors for the built UKI.
    pub sections: Vec<SectionInfo>,
    /// Overlay source information for deferred overlay pulling.
    pub overlay: Option<Overlay>,
}

static SOURCES: OnceLock<Sources> = OnceLock::new();

/// Configure global source addresses. Must be called once before building.
///
/// # Panics
///
/// Panics when called more than once.
pub fn configure(sources: Sources) {
    assert!(SOURCES.set(sources).is_ok(), "sources already configured");
}

/// Returns the globally configured sources.
///
/// # Errors
///
/// Returns an error when [`configure`] has not been called.
pub fn sources() -> Result<&'static Sources> {
    SOURCES.get().ok_or_else(|| {
        WizardError::BuildError("sources not configured; call build::configure() first".to_owned())
    })
}

/// Builds the requested artifacts from a resolved plan.
///
/// # Errors
///
/// Returns an error when pulling, building, or signing fails.
pub(crate) async fn execute(
    plan: &resolve::BuildPlan,
    profile: &Profile,
    targets: Vec<(Artifact, &mut dyn Write)>,
    signing_key: Option<&SigningPair<'_>>,
) -> Result<Metadata> {
    if targets.is_empty() {
        return Err(WizardError::BuildError(
            "at least one artifact must be requested".to_owned(),
        ));
    }

    let meta = installer::metadata(plan.installer(), &plan.arch(), None).await?;

    let needs_post = targets
        .iter()
        .any(|item| matches!(item.0, Artifact::Uki | Artifact::Iso | Artifact::Raw));
    let needs_tail = needs_post || targets.iter().any(|item| item.0 == Artifact::Initramfs);

    let extensions = if needs_tail {
        Some(extension::pull(plan.extensions(), &plan.arch()).await?)
    } else {
        None
    };

    let (tail_parts, tail_size) = if needs_tail {
        let parts = archive::prepare_tail_parts(
            extensions.as_deref().unwrap_or(&[]),
            &profile.canonical_bytes()?,
        )?;
        let size = archive::tail_exact_size(&parts);
        (Some(parts), size)
    } else {
        (None, 0)
    };

    let post_config = artifacts::BuildPostConfig {
        resolved: plan,
        installer_meta: &meta,
        tail_parts: tail_parts.as_ref(),
        tail_size,
        signing_key,
    };
    let sections = artifacts::build(post_config, targets).await?;

    let overlay = plan.overlay().cloned();

    Ok(Metadata {
        sections: sections
            .into_iter()
            .map(|section| SectionInfo {
                name: section.name.to_owned(),
                file_offset: section.file_offset,
                size: section.size,
                hash: section.checksum,
            })
            .collect(),
        overlay,
    })
}

/// Set the OCI blob cache directory for all image pulls performed by koci.
pub fn set_cache_dir<P: Into<PathBuf>>(path: P) {
    cache::Store::set_dir(path.into());
}
