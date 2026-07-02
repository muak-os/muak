//! OCI blob cache backed by the local filesystem.

use core::time::Duration;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::time::SystemTime;

/// Cache directory configuration.
static CACHE_DIR: OnceLock<Option<PathBuf>> = OnceLock::new();

/// Set the OCI blob cache directory for all subsequent pulls.
///
/// Must be called before any pull operation. Thread-safe and idempotent —
/// only the first call takes effect.
pub fn set_dir<P: Into<PathBuf>>(path: P) {
    drop(CACHE_DIR.set(Some(path.into())));
}

/// A local filesystem cache for OCI blobs and tag-to-manifest mappings.
#[derive(Clone)]
pub(crate) struct BlobCache {
    root: Option<PathBuf>,
    ttl: Duration,
}

impl BlobCache {
    /// Create a new cache.
    ///
    /// Resolution order:
    /// 1. `MUAK_KOCI_CACHE` environment variable (highest priority)
    /// 2. [`set_dir`]
    /// 3. No cache (all methods become no-ops)
    pub fn new() -> Self {
        let root = std::env::var("MUAK_KOCI_CACHE")
            .ok()
            .map(PathBuf::from)
            .or_else(|| CACHE_DIR.get().cloned().flatten());
        let ttl = std::env::var("MUAK_KOCI_CACHE_TTL")
            .ok()
            .and_then(|value| value.parse::<u64>().ok())
            .map_or(Duration::from_mins(5), Duration::from_secs);

        Self { root, ttl }
    }

    /// Return cached blob bytes for the given digest, or `None`.
    pub fn get_blob(&self, digest: &str) -> Option<Vec<u8>> {
        let path = self.blob_path(digest)?;

        std::fs::read(&path).ok()
    }

    /// Store blob bytes for the given digest.
    ///
    /// Writes atomically via a temporary file followed by rename.
    pub fn put_blob(&self, digest: &str, data: &[u8]) {
        let Some(path) = self.blob_path(digest) else {
            return;
        };
        atomic_write(&path, data);
    }

    /// Return cached manifest JSON for a tag reference, or `None` if
    /// missing or the TTL has expired.
    pub fn get_ref(&self, registry: &str, name: &str, tag: &str) -> Option<String> {
        let path = self.ref_path(registry, name, tag)?;
        let metadata = std::fs::metadata(&path).ok()?;

        let modified = metadata.modified().ok()?;
        if SystemTime::now()
            .duration_since(modified)
            .unwrap_or(Duration::MAX)
            > self.ttl
        {
            drop(std::fs::remove_file(&path));
            return None;
        }

        std::fs::read_to_string(&path).ok()
    }

    /// Store manifest JSON for a tag reference.
    pub fn put_ref(&self, registry: &str, name: &str, tag: &str, manifest: &str) {
        let Some(path) = self.ref_path(registry, name, tag) else {
            return;
        };
        atomic_write(&path, manifest.as_bytes());
    }

    fn blob_path(&self, digest: &str) -> Option<PathBuf> {
        let root = self.root.as_ref()?;
        let hash = digest.strip_prefix("sha256:")?;

        Some(root.join("blobs").join("sha256").join(hash))
    }

    fn ref_path(&self, registry: &str, name: &str, tag: &str) -> Option<PathBuf> {
        let root = self.root.as_ref()?;

        Some(root.join("refs").join(registry).join(name).join(tag))
    }
}

/// Write `data` to `path` atomically using a temporary file + rename.
fn atomic_write(path: &Path, data: &[u8]) {
    let Some(parent) = path.parent() else {
        return;
    };
    if std::fs::create_dir_all(parent).is_err() {
        return;
    }
    let tmp = path.with_extension("tmp");
    if std::fs::write(&tmp, data).is_err() {
        return;
    }

    drop(std::fs::rename(&tmp, path));
}

#[cfg(test)]
mod tests {
    use std::thread;

    use tempfile::TempDir;

    use super::*;

    fn new_cache(dir: &TempDir) -> BlobCache {
        BlobCache {
            root: Some(dir.path().to_path_buf()),
            ttl: Duration::from_mins(5),
        }
    }

    #[test]
    fn blob_roundtrip() {
        // ARRANGE
        let tmp = TempDir::new().expect("temp dir");
        let cache = new_cache(&tmp);
        let digest = "sha256:abcd1234";
        let data = b"hello blob";

        // ACT
        cache.put_blob(digest, data);
        let got = cache.get_blob(digest).expect("should find blob");

        // ASSERT
        assert_eq!(got, data);
    }

    #[test]
    fn blob_not_found_returns_none() {
        // ARRANGE
        let tmp = TempDir::new().expect("temp dir");
        let cache = new_cache(&tmp);

        // ACT / ASSERT
        assert!(cache.get_blob("sha256:nonexistent").is_none());
    }

    #[test]
    fn blob_without_sha256_prefix_returns_none() {
        // ARRANGE
        let tmp = TempDir::new().expect("temp dir");
        let cache = new_cache(&tmp);
        cache.put_blob("sha256:abc", b"data");

        // ACT / ASSERT
        assert!(cache.get_blob("sha512:abc").is_none());
    }

    #[test]
    fn ref_roundtrip() {
        // ARRANGE
        let tmp = TempDir::new().expect("temp dir");
        let cache = new_cache(&tmp);
        let manifest = r#"{"schemaVersion":2}"#;

        // ACT
        cache.put_ref("ghcr.io", "org/image", "v1.0", manifest);
        let got = cache
            .get_ref("ghcr.io", "org/image", "v1.0")
            .expect("should find ref");

        // ASSERT
        assert_eq!(got, manifest);
    }

    #[test]
    fn ref_expires_after_ttl() {
        // ARRANGE
        let tmp = TempDir::new().expect("temp dir");
        let cache = BlobCache {
            root: Some(tmp.path().to_path_buf()),
            ttl: Duration::from_millis(10),
        };
        cache.put_ref("ghcr.io", "org/image", "latest", "{}");

        // ACT
        thread::sleep(Duration::from_millis(20));

        // ASSERT
        assert!(cache.get_ref("ghcr.io", "org/image", "latest").is_none());
    }

    #[test]
    fn no_cache_dir_all_methods_are_noops() {
        // ARRANGE
        let cache = BlobCache {
            root: None,
            ttl: Duration::from_mins(5),
        };

        // ACT
        cache.put_blob("sha256:abc", b"data");
        cache.put_ref("r", "n", "t", "{}");

        // ASSERT
        assert!(cache.get_blob("sha256:abc").is_none());
        assert!(cache.get_ref("r", "n", "t").is_none());
    }

    #[test]
    fn atomic_write_cleans_up_tmp_file() {
        // ARRANGE
        let tmp = TempDir::new().expect("temp dir");
        let path = tmp.path().join("test-file");

        // ACT
        atomic_write(&path, b"hello");

        // ASSERT
        assert!(path.exists());
        assert!(!path.with_extension("tmp").exists());
    }

    #[test]
    fn set_cache_dir_takes_effect() {
        // ARRANGE
        let _: Option<()> = CACHE_DIR.set(Some(PathBuf::from("/some/cache"))).ok();

        // ACT
        let cache = BlobCache::new();

        // ASSERT
        assert_eq!(cache.root, Some(PathBuf::from("/some/cache")));
    }
}
