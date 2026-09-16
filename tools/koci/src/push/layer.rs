//! Deterministic tar layer assembly for pushed images.

use std::fs::File;

use hyper::body::Bytes;
use tar::{Builder, Header};

use super::{Blob, Entry};
use crate::digest::sha256_hex;
use crate::error::Result;

/// Build the deterministic uncompressed tar layer holding all entries.
///
/// # Errors
///
/// Returns an error when a source file cannot be read or archiving fails.
pub(crate) fn build(entries: &[Entry]) -> Result<Blob> {
    let mut sorted: Vec<&Entry> = entries.iter().collect();
    sorted.sort_unstable_by(|left, right| left.path.cmp(&right.path));

    let mut builder = Builder::new(Vec::new());
    for entry in &sorted {
        append(&mut builder, entry)?;
    }
    let bytes = builder.into_inner()?;
    let digest = format!("sha256:{}", sha256_hex(&bytes));

    Ok(Blob {
        digest,
        size: super::blob_size(bytes.len())?,
        bytes: Bytes::from(bytes),
    })
}

fn append(builder: &mut Builder<Vec<u8>>, entry: &Entry) -> Result<()> {
    let mut file = File::open(&entry.source)?;
    let mut header = Header::new_gnu();
    header.set_size(file.metadata()?.len());
    header.set_mode(0o644);
    header.set_mtime(0);
    header.set_uid(0);
    header.set_gid(0);
    builder.append_data(&mut header, &entry.path, &mut file)?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use tar::Archive;
    use tempfile::TempDir;

    use super::*;

    fn entry_in(dir: &TempDir, name: &str, contents: &[u8]) -> Entry {
        let source = dir.path().join(name);
        std::fs::write(&source, contents).expect("write test entry file");

        Entry {
            path: name.to_owned(),
            source,
        }
    }

    fn archived_paths(bytes: &[u8]) -> Vec<String> {
        let mut archive = Archive::new(bytes);
        let mut paths = Vec::new();
        for entry in archive.entries().expect("iterate archive") {
            paths.push(
                entry
                    .expect("read archive entry")
                    .path()
                    .expect("entry path")
                    .to_string_lossy()
                    .to_string(),
            );
        }

        paths
    }

    #[test]
    fn build_sorts_entries_and_stays_deterministic() {
        // ARRANGE
        let workspace = TempDir::new().expect("create temp dir");
        let first = entry_in(&workspace, "a.toml", b"first");
        let second = entry_in(&workspace, "b.toml", b"second");

        // ACT
        let forward = build(&[first.clone(), second.clone()]).expect("build layer");
        let backward = build(&[second, first]).expect("build layer");

        // ASSERT
        assert_eq!(forward.bytes, backward.bytes);
        assert_eq!(forward.digest, backward.digest);
        assert_eq!(archived_paths(&forward.bytes), vec!["a.toml", "b.toml"]);
    }

    #[test]
    fn build_records_entry_sizes_and_zeroed_metadata() {
        // ARRANGE
        let workspace = TempDir::new().expect("create temp dir");
        let entry = entry_in(&workspace, "data.bin", &[0_u8; 11]);

        // ACT
        let blob = build(std::slice::from_ref(&entry)).expect("build layer");
        let mut archive = Archive::new(blob.bytes.as_ref());
        let mut entries = archive.entries().expect("iterate archive");
        let owned = entries.next().expect("first entry").expect("read entry");
        let header = owned.header();

        // ASSERT
        assert_eq!(header.size().ok(), Some(11));
        assert_eq!(header.mode().ok(), Some(0o644));
        assert_eq!(header.mtime().ok(), Some(0));
    }

    #[test]
    fn build_reports_missing_source_files() {
        // ARRANGE
        let missing = Entry {
            path: "ghost.toml".to_owned(),
            source: std::path::PathBuf::from("/nonexistent/ghost.toml"),
        };

        // ACT / ASSERT
        let error = build(&[missing]).expect_err("missing source should fail");
        assert!(matches!(error, crate::error::KociError::IoError(_)));
    }
}
