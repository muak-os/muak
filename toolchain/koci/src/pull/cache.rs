//! OCI blob cache backed by the local filesystem.

use std::collections::HashSet;
use std::hash::BuildHasher;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::sync::atomic::{AtomicU8, AtomicUsize, Ordering};
use std::time::SystemTime;

use oci::digest::sha256_hex;
use oci::reference::Image;
use rustix::fs::XattrFlags;
use rustix::io::Errno;

/// Subtree holding in-progress ingests, shared with the content store.
pub const INGEST_DIR: &str = "ingest";

const REFS_DIR: &str = "refs";
const INDEX_DIR: &str = "index";
const BLOBS_DIR: &str = "blobs";
const SHA256_PREFIX: &str = "sha256:";
const SHA256_ALGORITHM: &str = "sha256";
const USE_COUNT_XATTR: &str = "user.muak.use_count";

const XATTR_UNKNOWN: u8 = 0;
const XATTR_SUPPORTED: u8 = 1;
const XATTR_UNSUPPORTED: u8 = 2;

static XATTR_STATE: AtomicU8 = AtomicU8::new(XATTR_UNKNOWN);

static CACHE_DIR: OnceLock<Option<PathBuf>> = OnceLock::new();

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
        Self::at(root())
    }

    /// Create a cache over `root`, or a disabled cache when it is `None`.
    #[must_use]
    pub(crate) fn at(root: Option<PathBuf>) -> Self {
        Self { root }
    }

    /// Return a cached manifest.
    pub(crate) fn get_manifest(&self, image: &Image, manifest_ref: &str) -> Option<String> {
        if manifest_ref.starts_with(SHA256_PREFIX) {
            let path = self.blob_path(manifest_ref)?;
            let manifest = std::fs::read_to_string(&path).ok()?;
            touch(&path);
            return Some(manifest);
        }

        self.get_ref(&image.registry, &image.name, manifest_ref)
    }

    /// Store a manifest and anchor its digest for cache retention.
    pub(crate) fn put_manifest(&self, image: &Image, manifest_ref: &str, manifest: &str) {
        if manifest_ref.starts_with(SHA256_PREFIX) {
            self.put_blob(manifest_ref, manifest.as_bytes());
            self.put_ref(&image.registry, &image.name, manifest_ref, manifest);
            self.put_index(manifest_ref, manifest);
        } else {
            self.put_ref(&image.registry, &image.name, manifest_ref, manifest);
            let digest = format!("{SHA256_PREFIX}{}", sha256_hex(manifest.as_bytes()));
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

    fn get_ref(&self, registry: &str, name: &str, tag: &str) -> Option<String> {
        let path = self.ref_path(registry, name, tag)?;
        let manifest = std::fs::read_to_string(&path).ok()?;
        touch(&path);

        Some(manifest)
    }

    fn put_ref(&self, registry: &str, name: &str, tag: &str, manifest: &str) {
        let Some(path) = self.ref_path(registry, name, tag) else {
            return;
        };
        atomic_write(&path, manifest.as_bytes());
    }

    fn put_index(&self, manifest_digest: &str, manifest: &str) {
        let Some(root) = self.root.as_ref() else {
            return;
        };
        let Some(hash) = manifest_digest.strip_prefix(SHA256_PREFIX) else {
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

        Some(root.join(REFS_DIR).join(registry).join(name).join(tag))
    }
}

/// Every digest reachable from the tag references under `root`.
#[must_use]
pub fn shared_digests(root: &Path) -> HashSet<String> {
    let mut digests = HashSet::new();
    let mut pending = Vec::new();
    ref_manifest_digests(&root.join(REFS_DIR), &mut pending);

    while let Some(digest) = pending.pop() {
        if digests.contains(&digest) {
            continue;
        }
        pending.extend(index_digests(root, &digest));
        digests.insert(digest);
    }

    digests
}

/// Whether `path` holds cache metadata that every cached image depends on.
#[must_use]
pub fn is_metadata(root: &Path, path: &Path) -> bool {
    matches!(subtree(root, path), Some(REFS_DIR | INDEX_DIR))
}

/// Whether `path` is still referenced by something kept in the cache.
#[must_use]
pub fn is_shared<S: BuildHasher>(root: &Path, path: &Path, shared: &HashSet<String, S>) -> bool {
    if matches!(subtree(root, path), Some(REFS_DIR)) {
        return true;
    }
    let Some(hash) = path.file_name().and_then(|name| name.to_str()) else {
        return false;
    };
    let digest = format!("{SHA256_PREFIX}{hash}");
    if !shared.contains(&digest) {
        return false;
    }

    match subtree(root, path) {
        Some(INDEX_DIR) => root.join(INDEX_DIR).join(hash) == path,
        Some(BLOBS_DIR) => blob_file_path(Some(root), &digest).as_deref() == Some(path),
        _ => false,
    }
}

/// The recorded use count of a cache entry, or `0` when it carries none.
#[must_use]
pub fn use_count(path: &Path) -> u64 {
    if unsupported() {
        return 0;
    }

    let mut value = [0_u8; 8];
    let len = match rustix::fs::getxattr(path, USE_COUNT_XATTR, &mut value[..]) {
        Ok(len) => {
            XATTR_STATE.store(XATTR_SUPPORTED, Ordering::Relaxed);
            len
        }
        Err(errno) => {
            record(errno);
            return 0;
        }
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
    if unsupported() {
        return;
    }

    let count = use_count(path).saturating_add(1);
    if unsupported() {
        return;
    }

    match rustix::fs::setxattr(
        path,
        USE_COUNT_XATTR,
        &count.to_le_bytes(),
        XattrFlags::empty(),
    ) {
        Ok(()) => XATTR_STATE.store(XATTR_SUPPORTED, Ordering::Relaxed),
        Err(errno) => record(errno),
    }
}

/// The resolved cache root, or `None` when both caches are disabled.
#[must_use]
pub fn root() -> Option<PathBuf> {
    std::env::var("MUAK_KOCI_CACHE")
        .ok()
        .map(PathBuf::from)
        .or_else(|| CACHE_DIR.get().cloned().flatten())
}

/// The filesystem path for a committed blob digest.
pub(crate) fn blob_file_path(root: Option<&Path>, digest: &str) -> Option<PathBuf> {
    let hash = digest.strip_prefix(SHA256_PREFIX)?;

    Some(root?.join(BLOBS_DIR).join(SHA256_ALGORITHM).join(hash))
}

fn unsupported() -> bool {
    XATTR_STATE.load(Ordering::Relaxed) == XATTR_UNSUPPORTED
}

fn classify(errno: Errno) -> Option<u8> {
    match errno {
        Errno::NODATA => Some(XATTR_SUPPORTED),
        Errno::NOTSUP => Some(XATTR_UNSUPPORTED),
        _ => None,
    }
}

fn record(errno: Errno) {
    if let Some(state) = classify(errno) {
        XATTR_STATE.store(state, Ordering::Relaxed);
    }
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

fn ref_manifest_digests(dir: &Path, digests: &mut Vec<String>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };

    for entry in entries.flatten() {
        let path = entry.path();
        let Ok(file_type) = entry.file_type() else {
            continue;
        };
        if file_type.is_dir() {
            ref_manifest_digests(&path, digests);
            continue;
        }
        let Ok(bytes) = std::fs::read(&path) else {
            continue;
        };
        digests.push(format!("{SHA256_PREFIX}{}", sha256_hex(&bytes)));
    }
}

fn index_digests(root: &Path, digest: &str) -> Vec<String> {
    let Some(hash) = digest.strip_prefix(SHA256_PREFIX) else {
        return Vec::new();
    };
    let Ok(text) = std::fs::read_to_string(root.join(INDEX_DIR).join(hash)) else {
        return Vec::new();
    };

    text.lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(str::to_owned)
        .collect()
}

fn subtree<'a>(root: &Path, path: &'a Path) -> Option<&'a str> {
    let relative = path.strip_prefix(root).ok()?;

    relative.components().next()?.as_os_str().to_str()
}

/// Write `data` to `path` via a temp sibling, so a crash never leaves a partial file.
pub(crate) fn atomic_write(path: &Path, data: &[u8]) {
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
    fn classify_missing_attribute() {
        // ARRANGE
        let errno = Errno::NODATA;

        // ACT
        let outcome = classify(errno);

        // ASSERT
        assert_eq!(outcome, Some(XATTR_SUPPORTED));
    }

    #[test]
    fn classify_unsupported_attribute() {
        // ARRANGE
        let errno = Errno::NOTSUP;

        // ACT
        let outcome = classify(errno);

        // ASSERT
        assert_eq!(outcome, Some(XATTR_UNSUPPORTED));
    }

    #[test]
    fn use_count_without_xattr_is_zero() {
        // ARRANGE
        let tmp = TempDir::new().expect("temp dir");
        let path = tmp.path().join("entry");
        std::fs::write(&path, b"data").expect("write entry");

        // ACT
        let count = use_count(&path);

        // ASSERT
        assert_eq!(count, 0, "an entry without the xattr has no recorded count");
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
    fn tag_ref_reads_refresh_mtime_and_use_count() {
        // ARRANGE
        let tmp = TempDir::new().expect("temp dir");
        let cache = new_cache(&tmp);
        let image = Image::parse("ghcr.io/org/image:v1");
        cache.put_manifest(&image, "v1", "{}");
        let path = cache
            .ref_path("ghcr.io", "org/image", "v1")
            .expect("ref path");
        let old = SystemTime::now()
            .checked_sub(std::time::Duration::from_hours(2))
            .expect("old timestamp");
        let file = std::fs::File::options()
            .write(true)
            .open(&path)
            .expect("open ref");
        file.set_modified(old).expect("set mtime");
        drop(file);

        // ACT
        let cached = cache.get_manifest(&image, "v1");

        // ASSERT
        let modified = std::fs::metadata(&path)
            .expect("ref meta")
            .modified()
            .expect("ref mtime");
        assert_eq!(cached.as_deref(), Some("{}"));
        assert!(modified > old, "a tag hit must refresh the ref mtime");
        assert_eq!(use_count(&path), 1, "a tag hit must bump the ref use count");
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
    fn shared_digests_follows_tag_refs() {
        // ARRANGE
        let tmp = TempDir::new().expect("temp dir");
        let cache = new_cache(&tmp);
        let image = Image::parse("ghcr.io/org/image:v1");
        let manifest = r#"{"schemaVersion":2,"layers":[{"digest":"sha256:layer"}]}"#;
        let ref_digest = format!("sha256:{}", sha256_hex(manifest.as_bytes()));

        // ACT
        cache.put_manifest(&image, "v1", manifest);
        let digests = shared_digests(tmp.path());

        // ASSERT
        assert!(
            digests.contains(&ref_digest),
            "the digest of a tag ref manifest is shared"
        );
        assert!(
            digests.contains("sha256:layer"),
            "referenced digests are shared"
        );
    }

    #[test]
    fn shared_digests_skips_orphan_index_files() {
        // ARRANGE
        let tmp = TempDir::new().expect("temp dir");
        let cache = new_cache(&tmp);
        let manifest = r#"{"schemaVersion":2,"layers":[{"digest":"sha256:orphaned"}]}"#;

        // ACT
        cache.put_index("sha256:orphan", manifest);
        let digests = shared_digests(tmp.path());
        let path = tmp.path().join("index/orphan");

        // ASSERT
        assert!(
            !digests.contains("sha256:orphan"),
            "an index file no anchored manifest reaches must not share its digest"
        );
        assert!(
            !is_shared(tmp.path(), &path, &digests),
            "an orphan index file is unshared"
        );
    }

    #[test]
    fn deleting_the_tag_ref_unshares_everything() {
        // ARRANGE
        let tmp = TempDir::new().expect("temp dir");
        let cache = new_cache(&tmp);
        let image = Image::parse("ghcr.io/org/image:v1");
        let manifest = r#"{"schemaVersion":2,"layers":[{"digest":"sha256:layer"}]}"#;
        cache.put_manifest(&image, "v1", manifest);
        let ref_path = cache
            .ref_path("ghcr.io", "org/image", "v1")
            .expect("ref path");

        // ACT
        std::fs::remove_file(&ref_path).expect("remove ref");
        let digests = shared_digests(tmp.path());

        // ASSERT
        assert!(
            digests.is_empty(),
            "without a tag ref nothing is shared any more"
        );
    }

    #[test]
    fn shared_digests_follows_a_manifest_list_child() {
        // ARRANGE
        let tmp = TempDir::new().expect("temp dir");
        let cache = new_cache(&tmp);
        let image = Image::parse("ghcr.io/org/image:v1");
        let child = r#"{"schemaVersion":2,"layers":[{"digest":"sha256:childlayer"}]}"#;
        let child_digest = format!("sha256:{}", sha256_hex(child.as_bytes()));
        cache.put_manifest(&image, &child_digest, child);
        let list = format!(r#"{{"schemaVersion":2,"manifests":[{{"digest":"{child_digest}"}}]}}"#);

        // ACT
        cache.put_manifest(&image, "v1", &list);
        let digests = shared_digests(tmp.path());

        // ASSERT
        assert!(
            digests.contains(&child_digest),
            "the child manifest of a manifest list is shared"
        );
        assert!(
            digests.contains("sha256:childlayer"),
            "digests of the child manifest are shared"
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
    fn refs_are_shared_metadata() {
        // ARRANGE
        let tmp = TempDir::new().expect("temp dir");
        let path = tmp.path().join("refs/registry/image/tag");
        let shared = HashSet::new();

        // ACT
        let metadata = is_metadata(tmp.path(), &path);
        let kept = is_shared(tmp.path(), &path, &shared);

        // ASSERT
        assert!(metadata, "refs are cache metadata");
        assert!(kept, "refs are shared");
    }

    #[test]
    fn index_entries_are_shared_metadata() {
        // ARRANGE
        let tmp = TempDir::new().expect("temp dir");
        let path = tmp.path().join("index/abcd1234");
        let shared = HashSet::new();

        // ACT
        let metadata = is_metadata(tmp.path(), &path);
        let kept = is_shared(tmp.path(), &path, &shared);

        // ASSERT
        assert!(metadata, "index entries are cache metadata");
        assert!(
            !kept,
            "an index entry whose manifest digest is absent is unshared"
        );
    }

    #[test]
    fn blob_is_shared_when_its_digest_is_in_the_set() {
        // ARRANGE
        let tmp = TempDir::new().expect("temp dir");
        let path = tmp.path().join("blobs/sha256/abcd1234");
        let shared: HashSet<String> = ["sha256:abcd1234".to_owned()].into_iter().collect();

        // ACT
        let metadata = is_metadata(tmp.path(), &path);
        let kept = is_shared(tmp.path(), &path, &shared);

        // ASSERT
        assert!(!metadata, "blobs are not cache metadata");
        assert!(kept, "a blob with a shared digest is kept");
    }

    #[test]
    fn manifest_blob_layout_matches_is_shared() {
        // ARRANGE
        let tmp = TempDir::new().expect("temp dir");
        let cache = new_cache(&tmp);
        let image = Image::parse("ghcr.io/org/image:v1");
        let manifest = r#"{"schemaVersion":2,"layers":[{"digest":"sha256:layer"}]}"#;
        let digest = format!("sha256:{}", sha256_hex(manifest.as_bytes()));
        cache.put_manifest(&image, &digest, manifest);

        // ACT
        let path = cache.blob_path(&digest).expect("blob path");
        let shared = shared_digests(tmp.path());

        // ASSERT
        assert!(
            is_shared(tmp.path(), &path, &shared),
            "a manifest written by the store must be shared at its blob path"
        );
    }

    #[test]
    fn digest_pinned_pull_stays_shared() {
        // ARRANGE
        let tmp = TempDir::new().expect("temp dir");
        let cache = new_cache(&tmp);
        let image = Image::parse("ghcr.io/org/image@sha256:abc");
        let manifest = r#"{"schemaVersion":2,"config":{"digest":"sha256:cfg"},"layers":[{"digest":"sha256:layer"}]}"#;
        let digest = format!("sha256:{}", sha256_hex(manifest.as_bytes()));

        // ACT
        cache.put_manifest(&image, &digest, manifest);
        let digests = shared_digests(tmp.path());

        // ASSERT
        let blob = cache.blob_path(&digest).expect("blob path");
        let index = tmp
            .path()
            .join("index")
            .join(digest.trim_start_matches(SHA256_PREFIX));
        assert!(
            is_shared(tmp.path(), &blob, &digests),
            "a manifest pulled by digest is shared at its blob path"
        );
        assert!(
            is_shared(tmp.path(), &index, &digests),
            "the index entry of a digest-pinned manifest is shared"
        );
        assert!(
            digests.contains("sha256:cfg") && digests.contains("sha256:layer"),
            "the config and layers of a digest-pinned manifest are shared"
        );
    }

    #[test]
    fn blob_is_unshared_without_a_matching_digest() {
        // ARRANGE
        let tmp = TempDir::new().expect("temp dir");
        let path = tmp.path().join("blobs/sha256/abcd1234");
        let shared: HashSet<String> = ["sha256:other".to_owned()].into_iter().collect();

        // ACT
        let kept = is_shared(tmp.path(), &path, &shared);

        // ASSERT
        assert!(!kept, "a blob no manifest references is unshared");
    }

    #[test]
    fn unrelated_paths_are_not_metadata_nor_shared() {
        // ARRANGE
        let tmp = TempDir::new().expect("temp dir");
        let path = tmp.path().join("misc/abcd1234");
        let shared: HashSet<String> = ["sha256:abcd1234".to_owned()].into_iter().collect();

        // ACT
        let metadata = is_metadata(tmp.path(), &path);
        let kept = is_shared(tmp.path(), &path, &shared);

        // ASSERT
        assert!(!metadata, "only refs and index are cache metadata");
        assert!(!kept, "paths outside the layout are unshared");
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
