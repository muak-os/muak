//! Public artifact build API.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use sbolt::keys::SigningPair;
use sbolt::pe::signature;
use tokio::fs;

use crate::artifact::Artifact;
use crate::error::{ImagerError, Result};
use crate::profile::Profile;
use crate::request::Request;
use crate::resolve::{self, Config};

pub(crate) mod archive;
pub(crate) mod media;
pub(crate) mod pipeline;
pub(crate) mod stage;
pub(crate) mod uki;

/// PE section metadata needed for TPM PCR#11 prediction.
#[derive(Debug, Clone)]
pub struct SectionInfo {
    /// PE section name (e.g. ".linux", ".initrd", ".cmdline").
    pub name: &'static str,
    /// File offset of the section data within the PE image.
    pub file_offset: usize,
    /// Size of the section data in bytes.
    pub size: usize,
}

/// Builds the requested artifacts sharing a single resolution.
///
/// # Errors
///
/// Returns an error when resolution, pulling, building, or signing fails.
pub async fn artifacts(
    request: &Request,
    profile: &Profile,
    config: &Config,
    signing_key: Option<&SigningPair<'_>>,
    output_dir: &Path,
) -> Result<HashMap<Artifact, PathBuf>> {
    let resolved = resolve::profile(request, profile, &config.sources)?;
    let profile_bytes = profile.canonical_bytes()?;

    fs::create_dir_all(output_dir)
        .await
        .map_err(|e| ImagerError::BuildError(format!("create output dir: {e}")))?;

    pipeline::artifacts(
        &resolved,
        &request.artifacts,
        signing_key,
        &profile_bytes,
        output_dir,
    )
    .await
}

/// Build the UKI and return its signed (or unsigned) bytes, PE section metadata,
/// and ESP overlay files.
///
/// If `signing_key` is provided the UKI is Authenticode-signed before returning.
///
/// # Errors
///
/// Returns an error when resolution, pulling, building, or signing fails.
pub async fn prepare_uki(
    request: &Request,
    profile: &Profile,
    config: &Config,
    signing_key: Option<&SigningPair<'_>>,
) -> Result<(Vec<u8>, Vec<SectionInfo>, Vec<esp::EspFile>)> {
    let resolved = resolve::profile(request, profile, &config.sources)?;
    let profile_bytes = profile.canonical_bytes()?;
    let prepared = pipeline::prepare(&resolved, &profile_bytes).await?;
    let overlay_files = pipeline::pull_overlay_if_present(&resolved).await?;

    let uki_bytes = if let Some(key) = signing_key {
        let capacity = prepared.uki_bytes.len().saturating_add(8192);
        let mut signed = Vec::with_capacity(capacity);
        signature::sign(
            &prepared.uki_bytes,
            key.signer,
            key.certificate,
            &mut signed,
        )
        .map_err(|e| ImagerError::BuildError(format!("sign UKI: {e}")))?;
        signed
    } else {
        prepared.uki_bytes
    };

    let sections = prepared
        .sections
        .into_iter()
        .map(|section| SectionInfo {
            name: section.name,
            file_offset: section.file_offset,
            size: section.size,
        })
        .collect();

    Ok((uki_bytes, sections, overlay_files))
}
