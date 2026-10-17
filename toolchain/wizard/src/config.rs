//! Process-wide build cache configuration.

use std::path::PathBuf;

use koci::pull::cache::Store;

/// Configures the local OCI blob cache for builds in this process.
///
/// Only the first call takes effect. Later calls are ignored.
pub fn init_cache(dir: PathBuf) {
    Store::set_dir(dir);
}
