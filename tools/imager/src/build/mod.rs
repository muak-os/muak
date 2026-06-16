//! Public artifact build API.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use tokio::fs;
pub use yuki::section::Section;

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

/// Builds the requested artifacts sharing a single resolution.
///
/// # Errors
///
/// Returns an error when resolution, pulling, or building fails.
pub async fn artifacts(
    request: &Request,
    profile: &Profile,
    config: &Config,
    output_dir: &Path,
) -> Result<HashMap<Artifact, PathBuf>> {
    let resolved = resolve::profile(request, profile, &config.sources)?;
    let profile_bytes = profile.canonical_bytes()?;

    fs::create_dir_all(output_dir)
        .await
        .map_err(|e| ImagerError::BuildError(format!("create output dir: {e}")))?;

    pipeline::artifacts(&resolved, &request.artifacts, &profile_bytes, output_dir).await
}

/// Build the UKI and return its raw bytes, PE sections, and ESP overlay files.
///
/// # Errors
///
/// Returns an error when resolution, pulling, or building fails.
pub async fn prepare_uki(
    request: &Request,
    profile: &Profile,
    config: &Config,
) -> Result<(Vec<u8>, Vec<Section>, Vec<esp::EspFile>)> {
    let resolved = resolve::profile(request, profile, &config.sources)?;
    let profile_bytes = profile.canonical_bytes()?;
    let prepared = pipeline::prepare(&resolved, &profile_bytes).await?;
    let overlay_files = pipeline::pull_overlay_if_present(&resolved).await?;

    Ok((prepared.uki_bytes, prepared.sections, overlay_files))
}
