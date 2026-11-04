//! OCI blob cache backed by the local filesystem.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::SystemTime;

use oci::digest::sha256_hex;
use oci::reference::Image;
use rustix::fs::XattrFlags;

/// Subtree holding in-progress ingests, shared with the content store.
pub const INGEST_DIR: &str = "ingest";

// Subtree holding the digests each cached manifest references.
const INDEX_DIR: &str = "index";

// Extended attribute recording how often a cache entry was used.
const USE_COUNT_XATTR: &str = "user.muak.use_count";

/// Cache directory configuration.
static CACHE_DIR: OnceLock<Option<PathBuf>> = OnceLock::new();

/// Sequence number for unique temporary file names.
static TEMP_SEQ: AtomicUsize = AtomicUsize::new(0);

/// A local filesystem cache for OCI blobs and tag-to-manifest mappings.
#[derive(Clone)]
pub struct Store {
    root: Option<PathBuf>,
}

impl Store {
    /// Set the cache directory programmatically.
    pub fn set_dir(path: PathBuf) {
        drop(CACHE_DIR.set(Some(path)));
    }

    /// Create a new cache.
    pub(crate) fn new() -> Self {
        Self { root: root() }
    }

    /// Return a cached manifest.
    pub(crate) fn get_manifest(&self, image: &Image, manifest_ref: &str) -> Option<String> {
        if is_digest(manifest_ref) {
            let path = self.blob_path(manifest_ref)?;
            let manifest = std::fs::read_to_string(&path).ok()?;
            touch(&path);
            return Some(manifest);
        }

        self.get_ref(&image.registry, &image.name, manifest_ref)
    }

    /// Store a manifest as a blob for digest references and under the image's ref path for tag references.
    pub(crate) fn put_manifest(&self, image: &Image, manifest_ref: &str, manifest: &str) {
        if is_digest(manifest_ref) {
            self.put_blob(manifest_ref, manifest.as_bytes());
            self.put_index(manifest_ref, manifest);
        } else {
            self.put_ref(&image.registry, &image.name, manifest_ref, manifest);
            let digest = format!("sha256:{}", sha256_hex(manifest.as_bytes()));
            self.put_index(&digest, manifest);
        }
    }

    /// Store blob bytes atomically, so a crash never leaves a partial entry.
    pub(crate) fn put_blob(&self, digest: &str, data: &[u8]) {
        if let Some(path) = self.blob_path(digest) {
            atomic_write(&path, data);
        }
    }

    /// Return the filesystem path for a blob digest.
    pub(crate) fn blob_path(&self, digest: &str) -> Option<PathBuf> {
        blob_file_path(self.root.as_deref(), digest)
    }

    // Return cached manifest JSON for a tag reference.
    //
    // Tags are immutable by contract, so a cached ref never expires by time.
    // A moved tag is a registry contract violation; the stale ref is only
    // removed by the cache eviction policy or a cache wipe. Digest-keyed
    // manifests and blobs never expire because content addresses are
    // immutable.
    fn get_ref(&self, registry: &str, name: &str, tag: &str) -> Option<String> {
        let path = self.ref_path(registry, name, tag)?;
        let manifest = std::fs::read_to_string(&path).ok()?;
        bump_use_count(&path);

        Some(manifest)
    }

    /// Store manifest JSON for a tag reference.
    fn put_ref(&self, registry: &str, name: &str, tag: &str, manifest: &str) {
        let Some(path) = self.ref_path(registry, name, tag) else {
            return;
        };
        atomic_write(&path, manifest.as_bytes());
    }

    // Record every digest `manifest` references under `manifest_digest`.
    fn put_index(&self, manifest_digest: &str, manifest: &str) {
        let Some(root) = self.root.as_ref() else {
            return;
        };
        let Some(hash) = manifest_digest.strip_prefix("sha256:") else {
            return;
        };
        let Ok(parsed) = oci::manifest::parse(manifest) else {
            return;
        };
        let referenced = parsed
            .config
            .iter()
            .chain(&parsed.layers)
            .chain(&parsed.manifests);
        let mut body = String::new();
        for descriptor in referenced {
            body.push_str(&descriptor.digest);
            body.push('\n');
        }

        atomic_write(&root.join(INDEX_DIR).join(hash), body.as_bytes());
    }

    fn ref_path(&self, registry: &str, name: &str, tag: &str) -> Option<PathBuf> {
        let root = self.root.as_ref()?;

        Some(root.join("refs").join(registry).join(name).join(tag))
    }
}

/// Every digest referenced by the manifests indexed under `root`.
///
/// The index itself is advisory: unreadable entries are ignored.
#[must_use]
pub fn shared_digests(root: &Path) -> HashSet<String> {
    let mut digests = HashSet::new();
    let Ok(entries) = std::fs::read_dir(root.join(INDEX_DIR)) else {
        return digests;
    };

    for entry in entries.flatten() {
        let path = entry.path();
        let Ok(text) = std::fs::read_to_string(&path) else {
            continue;
        };
        if let Some(name) = path.file_name().and_then(|name| name.to_str()) {
            digests.insert(format!("sha256:{name}"));
        }
        for line in text.lines().map(str::trim).filter(|line| !line.is_empty()) {
            digests.insert(line.to_owned());
        }
    }

    digests
}

/// The recorded use count of a cache entry, or `0` when it carries none.
#[must_use]
pub fn use_count(path: &Path) -> u64 {
    let mut value = [0_u8; 8];
    let Ok(len) = rustix::fs::getxattr(path, USE_COUNT_XATTR, &mut value[..]) else {
        return 0;
    };

    match value
        .get(..len)
        .and_then(|bytes| <[u8; 8]>::try_from(bytes).ok())
    {
        Some(bytes) => u64::from_le_bytes(bytes),
        None => 0,
    }
}

/// Increment the recorded use count of a cache entry.
pub fn bump_use_count(path: &Path) {
    let count = use_count(path).saturating_add(1);

    // A filesystem that rejects xattrs leaves the entry uncounted.
    let _stored = rustix::fs::setxattr(
        path,
        USE_COUNT_XATTR,
        &count.to_le_bytes(),
        XattrFlags::empty(),
    );
}

/// The cache root shared by the manifest cache and the content store.
///
/// Resolution order:
/// 1. `MUAK_KOCI_CACHE` environment variable (highest priority)
/// 2. [`Store::set_dir`]
/// 3. `None`: both caches are disabled
pub(crate) fn root() -> Option<PathBuf> {
    std::env::var("MUAK_KOCI_CACHE")
        .ok()
        .map(PathBuf::from)
        .or_else(|| CACHE_DIR.get().cloned().flatten())
}

/// The filesystem path holding a committed blob, or `None` when `root` is
/// unset or the digest has no `sha256:` prefix.
pub(crate) fn blob_file_path(root: Option<&Path>, digest: &str) -> Option<PathBuf> {
    let hash = digest.strip_prefix("sha256:")?;

    Some(root?.join("blobs").join("sha256").join(hash))
}

/// Refresh the mtime and the use count of a cache entry after a use.
pub(crate) fn touch(path: &Path) {
    bump_use_count(path);
    let Ok(file) = std::fs::OpenOptions::new().write(true).open(path) else {
        return;
    };

    drop(file.set_modified(SystemTime::now()));
}

/// Return a unique sibling path for an in-progress write.
pub(crate) fn temp_sibling(path: &Path) -> PathBuf {
    let seq = TEMP_SEQ.fetch_add(1, Ordering::Relaxed);

    path.with_file_name(format!(".{}.part.{seq}", std::process::id()))
}

// Whether a manifest reference is a content digest rather than a tag.
fn is_digest(manifest_ref: &str) -> bool {
    manifest_ref.starts_with("sha256:")
}

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

#[cfg(test)]
mod tests {
    use tempfile::TempDir;

    use super::*;

    fn new_cache(dir: &TempDir) -> Store {
        Store {
            root: Some(dir.path().to_path_buf()),
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
    fn tag_refs_never_expire() {
        // ARRANGE
        let tmp = TempDir::new().expect("temp dir");
        let cache = new_cache(&tmp);
        let image = Image::parse("ghcr.io/org/image:v1.0");
        let manifest = r#"{"schemaVersion":2}"#;
        cache.put_manifest(&image, "v1.0", manifest);
        let path = cache
            .ref_path("ghcr.io", "org/image", "v1.0")
            .expect("ref path");
        let old = SystemTime::now()
            .checked_sub(std::time::Duration::from_hours(24 * 365))
            .expect("old timestamp");
        let file = std::fs::File::options()
            .write(true)
            .open(&path)
            .expect("open ref");
        file.set_modified(old).expect("set mtime");
        drop(file);

        // ACT
        let cached = cache.get_manifest(&image, "v1.0");

        // ASSERT
        assert_eq!(
            cached.as_deref(),
            Some(manifest),
            "immutable tag refs must not expire by time"
        );
    }

    #[test]
    fn no_cache_dir_all_methods_are_noops() {
        // ARRANGE
        let cache = Store { root: None };

        // ACT
        cache.put_blob("sha256:abc", b"data");
        cache.put_ref("r", "n", "t", "{}");
        cache.put_manifest(&Image::parse("ghcr.io/org/image:v1"), "v1", "{}");

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
        assert_eq!(leftovers, Vec::<String>::new());
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
    fn manifest_cache_routes_digest_and_tag_references() {
        // ARRANGE
        let tmp = TempDir::new().expect("temp dir");
        let cache = new_cache(&tmp);
        let image = Image::parse("ghcr.io/org/image:v1");

        // ACT
        cache.put_manifest(&image, "sha256:abc", r#"{"kind":"digest"}"#);
        cache.put_manifest(&image, "v1", r#"{"kind":"tag"}"#);

        // ASSERT
        assert_eq!(
            cache.get_manifest(&image, "sha256:abc").as_deref(),
            Some(r#"{"kind":"digest"}"#),
            "digest references must be cached as blobs"
        );
        assert_eq!(
            cache.get_manifest(&image, "v1").as_deref(),
            Some(r#"{"kind":"tag"}"#),
            "tag references must be cached under the ref path"
        );
    }

    #[test]
    fn touch_refreshes_mtime_and_bumps_the_use_count() {
        // ARRANGE
        let tmp = TempDir::new().expect("temp dir");
        let path = tmp.path().join("entry");
        std::fs::write(&path, b"data").expect("write entry");
        let old = SystemTime::now()
            .checked_sub(std::time::Duration::from_hours(2))
            .expect("old timestamp");
        let file = std::fs::File::options()
            .write(true)
            .open(&path)
            .expect("open entry");
        file.set_modified(old).expect("set mtime");
        drop(file);

        // ACT
        touch(&path);
        let count = use_count(&path);
        touch(&path);

        // ASSERT
        let modified = std::fs::metadata(&path)
            .expect("entry meta")
            .modified()
            .expect("entry mtime");
        assert!(modified > old, "touch must refresh the entry mtime");
        assert_eq!(
            use_count(&path),
            count.saturating_add(1),
            "every touch must bump the use count"
        );
    }

    #[test]
    fn manifest_reads_touch_digest_blobs_and_tag_refs() {
        // ARRANGE
        let tmp = TempDir::new().expect("temp dir");
        let cache = new_cache(&tmp);
        let image = Image::parse("ghcr.io/org/image:v1");
        cache.put_manifest(&image, "sha256:abc", "{}");
        cache.put_manifest(&image, "v1", "{}");
        let digest_path = cache.blob_path("sha256:abc").expect("blob path");
        let tag_path = cache
            .ref_path("ghcr.io", "org/image", "v1")
            .expect("ref path");

        // ACT
        cache.get_manifest(&image, "sha256:abc").expect("digest");
        cache.get_manifest(&image, "v1").expect("tag");

        // ASSERT
        assert_eq!(use_count(&digest_path), 1, "digest hits must be counted");
        assert_eq!(use_count(&tag_path), 1, "tag ref hits must be counted");
    }

    #[test]
    fn manifest_index_records_config_layers_and_manifests() {
        // ARRANGE
        let tmp = TempDir::new().expect("temp dir");
        let cache = new_cache(&tmp);
        let image = Image::parse("ghcr.io/org/image:v1");
        let manifest = r#"{
            "schemaVersion": 2,
            "config": {"digest": "sha256:cfg"},
            "layers": [{"digest": "sha256:layer"}],
            "manifests": [{"digest": "sha256:platform"}]
        }"#;

        // ACT
        cache.put_manifest(&image, "sha256:multi", manifest);
        let indexed = std::fs::read_to_string(tmp.path().join("index/multi")).expect("index file");

        // ASSERT
        assert_eq!(indexed, "sha256:cfg\nsha256:layer\nsha256:platform\n");
    }

    #[test]
    fn shared_digests_returns_indexed_digests() {
        // ARRANGE
        let tmp = TempDir::new().expect("temp dir");
        let cache = new_cache(&tmp);
        let image = Image::parse("ghcr.io/org/image:v1");
        let manifest = r#"{"schemaVersion":2,"layers":[{"digest":"sha256:layer"}]}"#;

        // ACT
        cache.put_manifest(&image, "sha256:multi", manifest);
        cache.put_manifest(&image, "v1", manifest);
        let digests = shared_digests(tmp.path());

        // ASSERT
        assert!(digests.contains("sha256:multi"), "index files are shared");
        assert!(
            digests.contains("sha256:layer"),
            "referenced digests are shared"
        );
    }

    #[test]
    fn shared_digests_ignores_a_missing_index() {
        // ARRANGE
        let tmp = TempDir::new().expect("temp dir");

        // ACT
        let digests = shared_digests(tmp.path());

        // ASSERT
        assert!(digests.is_empty());
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
