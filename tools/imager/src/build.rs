//! Public artifact build API.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crate::artifact::Artifact;
use crate::error::Result;
use crate::profile::Profile;
use crate::render;
use crate::request::Request;
use crate::resolve::{self, Config};
use crate::workspace;

/// Builds the requested artifacts sharing a single resolution and workspace.
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
