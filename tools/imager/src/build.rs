//! Public artifact build API.

use std::path::{Path, PathBuf};

use crate::error::Result;
use crate::profile::Profile;
use crate::render;
use crate::request::{Build, Resolve};
use crate::resolve;
use crate::source::Sources;

/// Build pipeline configuration.
#[derive(Debug, Clone)]
pub struct Config {
    pub sources: Sources,
    pub workspace_root: PathBuf,
}

/// Builds the requested artifact and its dependencies from a profile.
///
/// # Errors
///
/// Returns an error when resolution, pulling, or building fails.
pub async fn artifacts(
    request: &Build,
    profile: &Profile,
    config: &Config,
    output_dir: &Path,
) -> Result<()> {
    let resolve_request = Resolve {
        version: request.version.clone(),
        platform: request.platform,
        arch: request.arch,
    };
    let resolved = resolve::profile(&resolve_request, profile, &config.sources)?;
    render::build(&resolved, output_dir).await
}
