//! Process-wide build cache configuration.

use std::path::PathBuf;

use koci::pull;

/// Configures the local OCI blob cache for builds in this process.
///
/// Only the first call takes effect. Later calls are ignored.
pub fn init_cache(dir: PathBuf) {
    pull::cache::Store::set_dir(dir);
}
