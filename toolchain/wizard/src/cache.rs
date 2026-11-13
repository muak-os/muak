//! Cache primitives for the koci pull cache.

use std::collections::HashSet;
use std::hash::BuildHasher;
use std::path::{Path, PathBuf};

use koci::pull;

/// Subtree holding in-progress ingests, re-exported from the koci pull cache.
pub const INGEST_DIR: &str = pull::cache::INGEST_DIR;

/// Cache access strategy for pulls, as defined by the koci pull module.
pub type Strategy = pull::Strategy;

/// The resolved cache root, or `None` when the cache is disabled.
#[must_use = "callers must handle the cache being disabled"]
pub fn root() -> Option<PathBuf> {
    pull::cache::root()
}

/// Digests still referenced by images in the cache.
#[must_use = "ignoring it deletes digests other images still use"]
pub fn shared_digests(root: &Path) -> HashSet<String> {
    pull::cache::shared_digests(root)
}

/// Whether `path` holds cache metadata that every cached image depends on.
#[must_use = "callers must keep the metadata every image depends on"]
pub fn is_metadata(root: &Path, path: &Path) -> bool {
    pull::cache::is_metadata(root, path)
}

/// Whether `path` is still referenced by something kept in the cache.
#[must_use = "callers must keep entries the cache still references"]
pub fn is_shared<S: BuildHasher>(root: &Path, path: &Path, shared: &HashSet<String, S>) -> bool {
    pull::cache::is_shared(root, path, shared)
}

/// Number of times the file at `path` has been used.
#[must_use = "callers must keep entries that are still in use"]
pub fn use_count(path: &Path) -> u64 {
    pull::cache::use_count(path)
}

/// Records one more use of the file at `path`.
pub fn bump_use_count(path: &Path) {
    pull::cache::bump_use_count(path);
}
