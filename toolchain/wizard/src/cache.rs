//! Cache primitives for the koci pull cache.

use std::collections::HashSet;
use std::path::Path;

use koci::pull::cache::{
    INGEST_DIR as CACHE_INGEST_DIR, bump_use_count as cache_bump_use_count,
    shared_digests as cache_shared_digests, use_count as cache_use_count,
};

/// Subtree holding in-progress ingests, which the content store collects itself.
pub const INGEST_DIR: &str = CACHE_INGEST_DIR;

/// Digests still referenced by images in the cache.
#[must_use]
pub fn shared_digests(root: &Path) -> HashSet<String> {
    cache_shared_digests(root)
}

/// Number of times the file at `path` has been used.
#[must_use]
pub fn use_count(path: &Path) -> u64 {
    cache_use_count(path)
}

/// Records one more use of the file at `path`.
pub fn bump_use_count(path: &Path) {
    cache_bump_use_count(path);
}
