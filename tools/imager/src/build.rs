//! Public artifact build API.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crate::artifact::Artifact;
use crate::error::Result;
use crate::profile::Profile;
use crate::render;
use crate::request::{Build, Resolve};
use crate::resolve::{self, Config};
use crate::workspace;

/// Builds the requested artifacts sharing a single resolution and workspace.
///
/// # Errors
///
/// Returns an error when resolution, pulling, or building fails.
pub async fn artifacts(
    request: &Build,
    profile: &Profile,
    config: &Config,
    output_dir: &Path,
) -> Result<HashMap<Artifact, PathBuf>> {
    let resolve_request = Resolve {
        version: request.version.clone(),
        platform: request.platform,
        arch: request.arch,
    };
    let resolved = resolve::profile(&resolve_request, profile, &config.sources)?;
    let profile_bytes = profile.canonical_bytes()?;
    let workspace = workspace::unique(&config.workspace_root);

    render::artifacts(
        &resolved,
        &request.artifacts,
        &profile_bytes,
        output_dir,
        &workspace,
    )
    .await
}
