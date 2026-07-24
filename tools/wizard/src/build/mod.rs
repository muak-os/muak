//! Public artifact build API.

use std::io::Write;

use koci::arch::Arch;
use sbolt::keys::SigningPair;
use serde::{Deserialize, Serialize};
use yuki::pe::section::Section;

use crate::artifact::Artifact;
use crate::build::sources::overlay::OverlayPipes;
use crate::error::{Result, WizardError};
use crate::profile::Profile;
use crate::resolve;

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
    /// PE section name.
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
}

/// Builds the requested artifacts from a resolved plan.
///
/// # Errors
///
/// Returns an error when pulling, building, or signing fails.
pub(crate) async fn execute(
    plan: &resolve::BuildPlan,
    profile: &Profile,
    signing: Option<&SigningPair<'_>>,
    targets: Vec<(Artifact, &mut (dyn Write + Send))>,
) -> Result<Metadata> {
    if targets.is_empty() {
        return Err(WizardError::BuildError(
            "at least one artifact must be requested".to_owned(),
        ));
    }

    let mut needs_tail = false;
    let mut needs_uki = false;
    let mut needs_media = false;
    let mut needs_overlays = false;
    for &(artifact, _) in &targets {
        if matches!(
            artifact,
            Artifact::Initramfs | Artifact::Uki | Artifact::Iso | Artifact::Raw
        ) {
            needs_tail = true;
        }
        if matches!(artifact, Artifact::Uki | Artifact::Iso | Artifact::Raw) {
            needs_uki = true;
        }
        if matches!(artifact, Artifact::Iso | Artifact::Raw) {
            needs_media = true;
        }
        if artifact == Artifact::Overlays {
            needs_overlays = true;
        }
    }

    let meta = sources::meta::fetch(plan).await?;
    let profile_bytes = profile.canonical_bytes()?;

    let tail = if needs_tail {
        let ext_data = sources::extensions::fetch(plan).await?;
        Some(transforms::tail::build(&ext_data, &profile_bytes)?)
    } else {
        None
    };

    let mut uki = if needs_uki {
        let tail_pipe = tail
            .as_ref()
            .and_then(|tailed| tailed.reader.try_clone().ok());
        Some(transforms::uki::open(
            &meta,
            tail.as_ref().map_or(0, |tailed| tailed.size),
            tail_pipe,
            signing,
        )?)
    } else {
        None
    };

    // TODO: single overlay setup with single pulling
    let overlay = if needs_media {
        Some(sources::overlay::setup(plan).await?)
    } else {
        None
    };

    let overlay_tar = if needs_overlays {
        plan.overlay()
            .map(|ov| sources::overlay::setup_tar(ov))
            .transpose()?
    } else {
        None
    };

    let mut router = router::Router::new(targets);
    let stub_pipe = uki.as_mut().and_then(transforms::uki::Uki::stub_w);
    let data_pipe = uki.as_mut().and_then(transforms::uki::Uki::data_w);
    let tail_pipe = tail
        .as_ref()
        .and_then(|tailed| tailed.reader.try_clone().ok());

    sources::installer::pull(plan, &mut router, stub_pipe, data_pipe, tail_pipe).await?;

    let overlay_w = if needs_overlays {
        router.take(Artifact::Overlays)
    } else {
        None
    };

    let sections = if let Some(uki) = uki {
        finalize_uki(uki, router, overlay, plan.arch()).await?
    } else {
        Vec::new()
    };

    if let Some(mut tar) = overlay_tar {
        if let Some(w) = overlay_w {
            std::io::copy(&mut tar.reader, w)
                .map_err(|e| WizardError::BuildError(format!("write overlay tar: {e}")))?;
        }
        tar.handle
            .await
            .map_err(|e| WizardError::BuildError(format!("join overlay tar task: {e}")))??;
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
    })
}

async fn finalize_uki(
    uki: transforms::uki::Uki,
    mut router: router::Router<'_>,
    mut overlay_pipes: Option<OverlayPipes>,
    arch: Arch,
) -> Result<Vec<Section>> {
    let outcome = transforms::uki::collect(uki).await?;

    if let Some(w) = router.take(Artifact::Iso) {
        let mut ukic = outcome
            .reader
            .try_clone()
            .map_err(|e| WizardError::BuildError(format!("clone UKI reader: {e}")))?;
        let ov = overlay_pipes
            .as_mut()
            .ok_or_else(|| WizardError::BuildError("overlay required for ISO".to_owned()))?;
        outputs::iso::iso(arch, &mut ukic, outcome.size, ov, w)?;
    }
    if let Some(w) = router.take(Artifact::Raw) {
        let mut ukic = outcome
            .reader
            .try_clone()
            .map_err(|e| WizardError::BuildError(format!("clone UKI reader: {e}")))?;
        let ov = overlay_pipes
            .as_mut()
            .ok_or_else(|| WizardError::BuildError("overlay required for RAW".to_owned()))?;
        outputs::raw::raw(arch, &mut ukic, outcome.size, ov, w)?;
    }
    if let Some(w) = router.take(Artifact::Uki) {
        let mut ukic = outcome
            .reader
            .try_clone()
            .map_err(|e| WizardError::BuildError(format!("clone UKI reader: {e}")))?;
        outputs::uki::uki(&mut ukic, w)?;
    }

    if let Some(ov) = overlay_pipes {
        ov.join().await?;
    }

    Ok(outcome.sections)
}
