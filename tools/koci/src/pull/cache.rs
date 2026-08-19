//! OCI blob cache backed by the local filesystem.

use core::time::Duration;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::SystemTime;

/// Cache directory configuration.
static CACHE_DIR: OnceLock<Option<PathBuf>> = OnceLock::new();

/// Sequence number for unique temporary file names.
static TEMP_SEQ: AtomicUsize = AtomicUsize::new(0);

/// A local filesystem cache for OCI blobs and tag-to-manifest mappings.
#[derive(Clone)]
pub struct Store {
    root: Option<PathBuf>,
    ttl: Duration,
}

impl Store {
    /// Set the cache directory programmatically.
    ///
    /// This is overridden by the `MUAK_KOCI_CACHE` environment variable if set.
    /// Must be called before creating any `Store` instances.
    pub fn set_dir(path: PathBuf) {
        drop(CACHE_DIR.set(Some(path)));
    }

    /// Create a new cache.
    ///
    /// Resolution order:
    /// 1. `MUAK_KOCI_CACHE` environment variable (highest priority)
    /// 2. [`set_dir`]
    /// 3. No cache (all methods become no-ops)
    pub(crate) fn new() -> Self {
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

    /// Store blob bytes atomically, so a crash never leaves a partial entry.
    pub(crate) fn put_blob(&self, digest: &str, data: &[u8]) {
        if let Some(path) = self.blob_path(digest) {
            atomic_write(&path, data);
        }
    }

    /// Return the filesystem path for a blob digest.
    pub(crate) fn blob_path(&self, digest: &str) -> Option<PathBuf> {
        let root = self.root.as_ref()?;
        let hash = digest.strip_prefix("sha256:")?;

        Some(root.join("blobs").join("sha256").join(hash))
    }

    /// Return cached manifest JSON for a tag reference, or `None` if
    /// missing or the TTL has expired.
    pub(crate) fn get_ref(&self, registry: &str, name: &str, tag: &str) -> Option<String> {
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
    pub(crate) fn put_ref(&self, registry: &str, name: &str, tag: &str, manifest: &str) {
        let Some(path) = self.ref_path(registry, name, tag) else {
            return;
        };
        atomic_write(&path, manifest.as_bytes());
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
    let tmp = temp_sibling(path);
    if std::fs::write(&tmp, data).is_err() {
        return;
    }

    drop(std::fs::rename(&tmp, path));
}

/// Return a unique sibling path for an in-progress write.
pub(crate) fn temp_sibling(path: &Path) -> PathBuf {
    let seq = TEMP_SEQ.fetch_add(1, Ordering::Relaxed);

    path.with_file_name(format!(".{}.part.{seq}", std::process::id()))
}

#[cfg(test)]
mod tests {
    use std::thread;

    use tempfile::TempDir;

    use super::*;

    fn new_cache(dir: &TempDir) -> Store {
        Store {
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
        let got = std::fs::read(cache.blob_path(digest).expect("blob path")).expect("read blob");

        // ASSERT
        assert_eq!(got, data);
    }

    #[test]
    fn blob_not_found_returns_none() {
        // ARRANGE
        let tmp = TempDir::new().expect("temp dir");
        let cache = new_cache(&tmp);

        // ACT / ASSERT
        assert!(
            !cache
                .blob_path("sha256:nonexistent")
                .expect("blob path")
                .exists()
        );
    }

    #[test]
    fn blob_without_sha256_prefix_returns_none() {
        // ARRANGE
        let tmp = TempDir::new().expect("temp dir");
        let cache = new_cache(&tmp);
        let src = tmp.path().join("src");
        std::fs::write(&src, b"data").expect("write src");
        cache.put_blob("sha256:abc", b"data");

        // ACT / ASSERT
        assert!(cache.blob_path("sha512:abc").is_none());
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
        let cache = Store {
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
        let cache = Store {
            root: None,
            ttl: Duration::from_mins(5),
        };

        // ACT
        cache.put_blob("sha256:abc", b"data");
        cache.put_ref("r", "n", "t", "{}");

        // ASSERT
        assert!(cache.blob_path("sha256:abc").is_none());
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
        let leftovers: Vec<String> = tmp
            .path()
            .read_dir()
            .expect("read dir")
            .map(|entry| {
                entry
                    .expect("dir entry")
                    .file_name()
                    .to_string_lossy()
                    .into_owned()
            })
            .filter(|name| name.contains(".part."))
            .collect();
        assert!(leftovers.is_empty());
    }

    #[test]
    fn temp_sibling_names_are_unique() {
        // ARRANGE
        let path = Path::new("/cache/blobs/sha256/abc");

        // ACT
        let first = temp_sibling(path);
        let second = temp_sibling(path);

        // ASSERT
        assert_ne!(first, second);
        assert_eq!(first.parent(), Some(Path::new("/cache/blobs/sha256")));
        assert_eq!(second.parent(), Some(Path::new("/cache/blobs/sha256")));
    }

    #[test]
    fn put_blob_roundtrip() {
        // ARRANGE
        let tmp = TempDir::new().expect("temp dir");
        let cache = new_cache(&tmp);
        let digest = "sha256:abcd1234";

        // ACT
        cache.put_blob(digest, b"hello blob");

        // ASSERT
        let path = cache.blob_path(digest).expect("blob path");
        let got = std::fs::read(&path).expect("read blob");
        assert_eq!(got, b"hello blob");
        let files: Vec<String> = tmp
            .path()
            .read_dir()
            .expect("read dir")
            .map(|entry| {
                entry
                    .expect("dir entry")
                    .file_name()
                    .to_string_lossy()
                    .into_owned()
            })
            .collect();
        assert_eq!(files, vec!["blobs"]);
    }

    #[test]
    fn set_cache_dir_takes_effect() {
        // ARRANGE
        let _: Option<()> = CACHE_DIR.set(Some(PathBuf::from("/some/cache"))).ok();

        // ACT
        let cache = Store::new();

        // ASSERT
        assert_eq!(cache.root, Some(PathBuf::from("/some/cache")));
    }
}
