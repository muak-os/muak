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
use crate::source::overlay::Overlay;

pub(crate) mod archive;
pub(crate) mod fanout;
pub(crate) mod media;
pub(crate) mod outputs;
pub(crate) mod router;
pub(crate) mod sources;
pub(crate) mod transforms;
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

/// Set the OCI blob cache directory for all image pulls performed by koci.
pub fn set_cache_dir<P: Into<PathBuf>>(path: P) {
    cache::Store::set_dir(path.into());
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

    // Phase 1: fetch metadata + profile (needed before we know tail requirements)
    let meta = sources::meta::fetch(plan).await?;
    let profile_bytes = profile.canonical_bytes()?;

    // Determine needs from targets directly
    let needs_tail = targets.iter().any(|&(artifact, _)| {
        matches!(
            artifact,
            Artifact::Initramfs | Artifact::Uki | Artifact::Iso | Artifact::Raw
        )
    });
    let needs_uki = targets
        .iter()
        .any(|&(artifact, _)| matches!(artifact, Artifact::Uki | Artifact::Iso | Artifact::Raw));
    let needs_media = targets
        .iter()
        .any(|&(artifact, _)| matches!(artifact, Artifact::Iso | Artifact::Raw));

    // Build tail BEFORE router so references can be shared
    let tail = if needs_tail {
        let ext_data = sources::extensions::fetch(plan).await?;
        Some(transforms::tail::build(&ext_data, &profile_bytes)?)
    } else {
        None
    };

    let mut uki = if needs_uki {
        Some(transforms::uki::open(
            &meta,
            tail.as_ref().map_or(0, |tail_info| tail_info.size),
            tail.as_ref().map(|tail_info| &tail_info.parts),
            signing_key,
        )?)
    } else {
        None
    };

    let overlay = if needs_media {
        Some(sources::overlay::setup(plan).await?)
    } else {
        None
    };

    let mut router = router::Router::new(targets);
    let stub_pipe = uki.as_mut().and_then(transforms::uki::Uki::stub_w);
    let data_pipe = uki.as_mut().and_then(transforms::uki::Uki::data_w);
    let tail_parts = tail.as_ref().map(|tail_info| &tail_info.parts);

    sources::installer::pull(plan, &mut router, stub_pipe, data_pipe, tail_parts).await?;

    // ── Finalize ──

    let mut sections = Vec::new();

    if let Some(uki) = uki {
        let outcome = transforms::uki::collect(uki).await?;
        sections = outcome.sections;

        let mut overlay = overlay;

        let uki_reader = outcome.reader;

        if let Some(w) = router.take(Artifact::Iso) {
            let mut clone = uki_reader
                .try_clone()
                .map_err(|e| WizardError::BuildError(format!("clone UKI reader: {e}")))?;
            let ov = overlay
                .as_mut()
                .ok_or_else(|| WizardError::BuildError("overlay required for ISO".to_owned()))?;
            outputs::iso::iso(plan.arch(), &mut clone, outcome.size, ov, w)?;
        }
        if let Some(w) = router.take(Artifact::Raw) {
            let mut clone = uki_reader
                .try_clone()
                .map_err(|e| WizardError::BuildError(format!("clone UKI reader: {e}")))?;
            let ov = overlay
                .as_mut()
                .ok_or_else(|| WizardError::BuildError("overlay required for RAW".to_owned()))?;
            outputs::raw::raw(plan.arch(), &mut clone, outcome.size, ov, w)?;
        }
        if let Some(w) = router.take(Artifact::Uki) {
            let mut clone = uki_reader
                .try_clone()
                .map_err(|e| WizardError::BuildError(format!("clone UKI reader: {e}")))?;
            outputs::uki::uki(&mut clone, w)?;
        }

        if let Some(ov) = overlay {
            ov.join().await?;
        }
    }

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
        overlay: plan.overlay().cloned(),
    })
}
