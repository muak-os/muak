//! Public artifact build API.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use tokio::fs;

use crate::artifact::Artifact;
use crate::error::{ImagerError, Result};
use crate::profile::Profile;
use crate::render;
use crate::request::Request;
use crate::resolve::{self, Config};

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

    render::artifacts(&resolved, &request.artifacts, &profile_bytes, output_dir).await
}
