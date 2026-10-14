//! eStargz-compatible seekable gzip layers with a table of contents.

mod encode;
mod gnu;
pub mod ingest;
pub mod member;
pub mod toc;

use std::path::PathBuf;

use oci::digest::sha256_hex;
use sha2::{Digest as _, Sha256};

use self::toc::{Record, footer};
use crate::error::Result;

/// One entry the layer writer packs, read from a file or in-memory bytes.
#[derive(Debug, Clone)]
pub struct Entry {
    /// Tar path carrying its repository prefix, never a bare directory name.
    pub path: String,
    /// Permission bits of the regular file entry.
    pub mode: u32,
    /// Local source holding the entry bytes.
    pub source: Source,
}

/// Where an entry's bytes come from.
#[derive(Debug, Clone)]
pub enum Source {
    /// A local file streamed once.
    File(PathBuf),
    /// In-memory bytes, streamed through a slice.
    Bytes(Vec<u8>),
}

/// Assembled eStargz layer with every digest the manifest needs.
#[derive(Debug)]
pub struct LayerBlob {
    /// Blob bytes.
    pub bytes: Vec<u8>,
    /// Compressed blob digest, the layer descriptor digest.
    pub digest: String,
    /// Compressed blob size, the layer descriptor size.
    pub size: u64,
    /// Uncompressed tar digest, the config `diff_ids` entry.
    pub uncompressed: String,
    /// Uncompressed tar byte count.
    pub uncompressed_size: u64,
    /// SHA-256 of the TOC JSON payload, the `toc.digest` annotation value.
    pub toc_digest: String,
}

/// Serializes `entries` into one eStargz layer blob.
///
/// # Errors
///
/// Returns an error when the entry list is empty, a source cannot be read, or
/// tar or gzip streaming fails.
pub fn layer(entries: &[Entry]) -> Result<LayerBlob> {
    let plan = member::plan(entries)?;
    let mut bytes = Vec::new();
    let mut tar_digest = Sha256::new();
    let mut toc: Vec<Record> = Vec::new();

    for current in &plan {
        toc.push(member::stream(&mut bytes, &mut tar_digest, current)?);
    }

    assemble(bytes, &mut tar_digest, &toc)
}

/// Finalizes a blob by appending the TOC member and the stargz footer.
pub(crate) fn assemble(
    mut bytes: Vec<u8>,
    tar_digest: &mut Sha256,
    toc: &[Record],
) -> Result<LayerBlob> {
    let offset = u64::try_from(bytes.len()).unwrap_or(u64::MAX);
    let (toc_member, toc_digest) = member::toc_member(toc)?;
    member::stream(&mut bytes, tar_digest, &toc_member)?;
    let uncompressed_size = u64::try_from(bytes.len()).unwrap_or(u64::MAX);
    bytes.extend_from_slice(&footer(offset));

    Ok(LayerBlob {
        digest: format!("sha256:{}", sha256_hex(&bytes)),
        size: u64::try_from(bytes.len()).unwrap_or(u64::MAX),
        uncompressed: format!("sha256:{}", sha256_hex(&tar_digest.clone().finalize())),
        uncompressed_size,
        toc_digest,
        bytes,
    })
}

#[cfg(test)]
mod layer_tests {
    use super::*;

    #[test]
    fn layer_carries_every_entry() {
        // ARRANGE
        let dir = tempfile::TempDir::new().expect("temp");
        let source = dir.path().join("catalog.toml");
        std::fs::write(&source, b"api_version = 1\n").expect("write");
        let entries = vec![Entry {
            path: "catalog.toml".to_owned(),
            mode: 0o644,
            source: Source::File(source),
        }];

        // ACT
        let blob = layer(&entries).expect("layer");
        let names: Vec<String> =
            tar::Archive::new(flate2::read::MultiGzDecoder::new(blob.bytes.as_slice()))
                .entries()
                .expect("entries")
                .filter_map(std::result::Result::ok)
                .map(|entry| String::from_utf8_lossy(entry.path_bytes().as_ref()).into_owned())
                .collect();

        // ASSERT
        assert!(
            names.contains(&"catalog.toml".to_owned()),
            "missing member: {names:?}"
        );
    }
}
