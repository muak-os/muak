//! Global build configuration.

use std::path::PathBuf;
use std::sync::OnceLock;

use koci::pull::cache;

use crate::error::{Result, WizardError};

/// Build configuration used throughout the pipeline.
#[derive(Debug, Clone)]
pub struct Config {
    /// Local directory for caching OCI blobs. When `None`, OCI pulls are not cached.
    pub cache_dir: Option<PathBuf>,
    /// Registry prefix (host[:port][/namespace]) used to build all source references.
    pub registry: String,
}

static CONFIG: OnceLock<Config> = OnceLock::new();

/// Sets the global configuration. Must be called once before building.
///
/// # Errors
///
/// Returns an error when called more than once.
pub fn configure(config: Config) -> Result<()> {
    if let Some(ref dir) = config.cache_dir {
        cache::Store::set_dir(dir.clone());
    }
    CONFIG
        .set(config)
        .map_err(|_prev| WizardError::BuildError("config already configured".to_owned()))
}

/// Returns the global configuration.
///
/// # Errors
///
/// Returns an error when [`configure`] has not been called.
pub fn config() -> Result<&'static Config> {
    CONFIG.get().ok_or_else(|| {
        WizardError::BuildError("config not configured; call config::configure() first".to_owned())
    })
}
