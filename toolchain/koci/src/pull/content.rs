//! Content-addressed store for layer blobs.

use std::fs::File;
use std::io::{self, Write as _};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, SystemTime};

use oci::digest::Verifier;

use super::cache;
use crate::error::{KociError, Result};

/// Directory under the store root holding in-progress ingests.
const INGEST_DIR: &str = "ingest";

/// Age at which an orphaned ingest file is garbage collected.
const STALE_INGEST: Duration = Duration::from_mins(60);

/// Sequence number for unique ingest file names.
static INGEST_SEQ: AtomicUsize = AtomicUsize::new(0);

/// A content-addressed store holding layer blobs under `blobs/sha256/`.
#[derive(Clone, Debug)]
pub struct Content {
    root: Option<PathBuf>,
}

impl Content {
    /// Create a content store over the cache root shared with the manifest
    /// cache, and garbage collect stale ingest files.
    #[must_use]
    pub(crate) fn new() -> Self {
        let content = Self {
            root: cache::root(),
        };
        content.gc();

        content
    }

    /// The root directory backing the store, or `None` when disabled.
    #[must_use]
    pub(crate) fn disk(&self) -> Option<&Path> {
        self.root.as_deref()
    }

    /// The committed size of a blob, or `None` when it is not stored.
    pub(crate) fn has_blob(&self, digest: &str) -> Option<u64> {
        let path = cache::blob_file_path(self.root.as_deref(), digest)?;
        std::fs::metadata(path).ok().map(|metadata| metadata.len())
    }

    /// Open a committed blob for sequential streaming reads.
    ///
    /// # Errors
    ///
    /// Returns an IO error when the blob is not stored.
    pub(crate) fn open_blob(&self, digest: &str) -> io::Result<File> {
        let path = cache::blob_file_path(self.root.as_deref(), digest).ok_or_else(|| {
            io::Error::new(io::ErrorKind::InvalidInput, "digest without sha256 prefix")
        })?;

        File::open(path)
    }

    /// Start a verified ingest of `digest` into a unique staging file.
    ///
    /// # Errors
    ///
    /// Returns an error when the store is disabled or the digest has no `sha256:` prefix.
    pub(crate) fn blob_writer(&self, digest: &str) -> Result<Ingest> {
        let target = cache::blob_file_path(self.root.as_deref(), digest)
            .ok_or_else(|| KociError::Pull(format!("digest without sha256 prefix: {digest}")))?;
        let staging = self.ingest_path(digest)?;
        if let Some(parent) = staging.parent() {
            std::fs::create_dir_all(parent).map_err(KociError::IoError)?;
        }
        let file = File::create(&staging).map_err(KociError::IoError)?;

        Ok(Ingest {
            staging_path: staging,
            target_path: target,
            file,
            committed: false,
        })
    }

    fn ingest_path(&self, digest: &str) -> Result<PathBuf> {
        let root = self.root.as_ref().ok_or_else(|| {
            KociError::Pull("no content store configured for blob ingest".to_owned())
        })?;
        let seq = INGEST_SEQ.fetch_add(1, Ordering::Relaxed);
        let hash = digest.strip_prefix("sha256:").unwrap_or(digest);

        Ok(root.join(INGEST_DIR).join(format!("{hash}.{seq}.ingest")))
    }

    fn gc(&self) {
        let Some(ref root) = self.root else { return };
        let Ok(entries) = std::fs::read_dir(root.join(INGEST_DIR)) else {
            return;
        };
        for stale in entries
            .flatten()
            .map(|entry| entry.path())
            .filter(|path| stale_staging(path))
        {
            drop(std::fs::remove_file(stale));
        }
    }
}

fn stale_staging(path: &Path) -> bool {
    let Ok(metadata) = std::fs::metadata(path) else {
        return false;
    };
    let Ok(modified) = metadata.modified() else {
        return false;
    };

    SystemTime::now()
        .duration_since(modified)
        .unwrap_or_default()
        > STALE_INGEST
}

/// A streaming write end for a blob being fetched, with atomic commitment.
pub struct Ingest {
    staging_path: PathBuf,
    target_path: PathBuf,
    file: File,
    committed: bool,
}

impl Ingest {
    /// Verify the streamed bytes and atomically commit them into the store.
    ///
    /// # Errors
    ///
    /// Returns an error when the bytes do not match `digest`, the file metadata cannot be read,
    /// or the rename fails. The staging file is removed either way.
    pub(crate) fn commit(mut self, verifier: Verifier) -> Result<u64> {
        self.file.flush().map_err(KociError::IoError)?;
        let size = self.file.metadata().map_err(KociError::IoError)?.len();
        verifier.verify().map_err(KociError::Oci)?;
        if let Some(parent) = self.target_path.parent() {
            std::fs::create_dir_all(parent).map_err(KociError::IoError)?;
        }
        std::fs::rename(&self.staging_path, &self.target_path).map_err(KociError::IoError)?;
        self.committed = true;

        Ok(size)
    }
}

impl std::io::Write for Ingest {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.file.write(buf)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.file.flush()
    }
}

impl Drop for Ingest {
    fn drop(&mut self) {
        if !self.committed {
            drop(std::fs::remove_file(&self.staging_path));
        }
    }
}

#[cfg(test)]
mod tests {
    use std::io::Read as _;

    use oci::digest::sha256_hex;
    use tempfile::TempDir;

    use super::*;

    fn content_at(tmp: &TempDir) -> Content {
        Content {
            root: Some(tmp.path().to_path_buf()),
        }
    }

    fn digest_of(data: &[u8]) -> String {
        format!("sha256:{}", sha256_hex(data))
    }

    fn committed_store_with(tmp: &TempDir) -> (Content, String, Vec<u8>) {
        let data: Vec<u8> = vec![0xA5_u8; 10_000];
        let content = content_at(tmp);
        let digest = digest_of(&data);
        ingest_then_commit(&content, &digest, &data);

        (content, digest, data)
    }

    fn ingest_then_commit(content: &Content, digest: &str, data: &[u8]) {
        let mut ingest = content.blob_writer(digest).expect("start ingest");
        let verifier = ingest_and_verify(&mut ingest, digest, data);
        ingest.commit(verifier).expect("commit blob");
    }

    fn ingest_and_verify(ingest: &mut Ingest, digest: &str, data: &[u8]) -> Verifier {
        let mut verifier = Verifier::new(digest).expect("verifier");
        for chunk in data.chunks(4096) {
            verifier.update(chunk);
            ingest.write_all(chunk).expect("write bytes");
        }

        verifier
    }

    #[test]
    fn commit_moves_the_ingest_into_the_blobs_tree() {
        // ARRANGE
        let tmp = TempDir::new().expect("temp dir");
        let data = b"hello blob";
        let content = content_at(&tmp);
        let digest = digest_of(data);

        // ACT
        ingest_then_commit(&content, &digest, data);
        let none_left = std::fs::read_dir(tmp.path().join("ingest"))
            .expect("read ingest dir")
            .next()
            .is_none();

        // ASSERT
        assert_eq!(
            content.has_blob(&digest),
            Some(u64::try_from(data.len()).expect("size"))
        );
        assert!(none_left, "the ingest file must be gone");
    }

    #[test]
    fn committed_blobs_stream_back_unchanged() {
        // ARRANGE
        let tmp = TempDir::new().expect("temp dir");
        let (content, digest, data) = committed_store_with(&tmp);

        // ACT
        let mut bytes = Vec::new();
        content
            .open_blob(&digest)
            .expect("open blob")
            .read_to_end(&mut bytes)
            .expect("read blob");

        // ASSERT
        assert_eq!(bytes, data);
    }

    #[test]
    fn aborted_ingests_leave_no_files_behind() {
        // ARRANGE
        let tmp = TempDir::new().expect("temp dir");
        let content = content_at(&tmp);
        let digest = digest_of(b"never committed because the writer is dropped");

        // ACT
        drop(content.blob_writer(&digest).expect("start ingest"));

        // ASSERT
        assert!(content.has_blob(&digest).is_none());
        assert!(
            std::fs::read_dir(tmp.path().join("ingest"))
                .expect("read ingest dir")
                .next()
                .is_none(),
            "the ingest file must be cleaned up"
        );
    }

    #[test]
    fn digest_mismatch_rejects_the_ingest_and_cleans_up() {
        // ARRANGE
        let tmp = TempDir::new().expect("temp dir");
        let content = content_at(&tmp);
        let digest = digest_of(b"committed content");
        let other = digest_of(b"someone corrupting the stream");

        // ACT
        let mut ingest = content.blob_writer(&digest).expect("start ingest");
        ingest_and_verify(&mut ingest, &other, b"corrupted bytes");
        let error = ingest
            .commit(Verifier::new(&digest).expect("verifier"))
            .expect_err("commit must fail");

        // ASSERT
        assert!(
            error.to_string().contains("sha256:"),
            "commit must report the digest mismatch: {error}"
        );
        assert!(content.has_blob(&digest).is_none());
        assert!(content.has_blob(&other).is_none());
    }

    #[test]
    fn fresh_ingest_files_survive_garbage_collection() {
        // ARRANGE
        let tmp = TempDir::new().expect("temp dir");
        let content = content_at(&tmp);
        let ingest = tmp.path().join("ingest/fresh.ingest");
        let stale = tmp.path().join("ingest/stale.ingest");
        std::fs::create_dir_all(tmp.path().join("ingest")).expect("create ingest dir");
        std::fs::write(&ingest, b"still being written").expect("write fresh");
        std::fs::write(&stale, b"abandoned").expect("write stale");
        let old = std::time::SystemTime::now()
            .checked_sub(STALE_INGEST + Duration::from_mins(1))
            .expect("old timestamp");
        let stale_file = File::options()
            .write(true)
            .open(&stale)
            .expect("open stale");
        stale_file.set_modified(old).expect("set mtime");
        drop(stale_file);

        // ACT
        content.gc();

        // ASSERT
        assert!(ingest.exists(), "fresh ingests must survive");
        assert!(!stale.exists(), "stale ingests must be collected");
    }

    #[test]
    fn disabled_content_stays_in_memory_mode() {
        // ARRANGE
        let content = Content { root: None };

        // ACT / ASSERT
        assert!(content.disk().is_none());
        assert!(content.has_blob("sha256:abc").is_none());
        assert!(
            content.blob_writer("sha256:abc").is_err(),
            "ingest must refuse when no store is configured"
        );
    }
}
