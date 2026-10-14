//! Low-level gzip member emission for eStargz layers.

use std::io::{self, Read, Write as _};

use flate2::Compression;
use flate2::write::GzEncoder;
use oci::digest::hex;
use sha2::{Digest as _, Sha256};
use tar::Header;

use super::toc::Record;
use crate::error::{KociError, Result};

/// Digest and byte length of the uncompressed tar stream written so far.
#[derive(Debug)]
pub(crate) struct Uncompressed {
    digest: Sha256,
    len: u64,
}

impl Uncompressed {
    /// Creates an empty accumulator.
    pub(crate) fn new() -> Self {
        Self {
            digest: Sha256::new(),
            len: 0,
        }
    }

    /// Returns the `sha256:` digest and byte length of the whole stream.
    pub(crate) fn finish(self) -> (String, u64) {
        (
            format!("sha256:{}", hex(self.digest.finalize().as_ref())),
            self.len,
        )
    }

    fn update(&mut self, buf: &[u8]) {
        self.digest.update(buf);
        self.len = self
            .len
            .saturating_add(u64::try_from(buf.len()).unwrap_or(u64::MAX));
    }
}

/// Streams one tar member, after raw `prefix` bytes, into its own gzip member.
///
/// # Errors
///
/// Returns an error when the header metadata is invalid or gzip writing fails.
pub(crate) fn emit(
    bytes: &mut Vec<u8>,
    uncompressed: &mut Uncompressed,
    prefix: &[u8],
    header: &Header,
    name: &str,
    payload: Option<&mut dyn Read>,
) -> Result<Record> {
    let offset = u64::try_from(bytes.len()).unwrap_or(u64::MAX);
    let size = header.size().map_err(KociError::IoError)?;
    let mode = header.mode().map_err(KociError::IoError)?;
    let kind = kind_of(header.entry_type());
    let mut payload_digest = Sha256::new();
    let mut encoded = GzEncoder::new(bytes, Compression::default());

    uncompressed.update(prefix);
    encoded.write_all(prefix).map_err(KociError::IoError)?;

    uncompressed.update(header.as_bytes());
    encoded
        .write_all(header.as_bytes())
        .map_err(KociError::IoError)?;

    if let Some(reader) = payload {
        stream_payload(&mut encoded, &mut payload_digest, uncompressed, reader)?;
    }
    pad_member(&mut encoded, uncompressed, size)?;

    let digest =
        (kind == "reg").then(|| format!("sha256:{}", hex(payload_digest.finalize().as_ref())));
    encoded
        .finish()
        .map_err(|error| KociError::PushError(format!("gzip member: {error}")))?;

    Ok(Record {
        name: name.to_owned(),
        kind: kind.to_owned(),
        size: (kind == "reg").then_some(size),
        mode: Some(u64::from(mode)),
        offset: Some(offset),
        digest: digest.clone(),
        chunk_digest: digest,
    })
}

fn kind_of(entry_type: tar::EntryType) -> &'static str {
    if entry_type.is_file() || entry_type.is_contiguous() {
        "reg"
    } else if entry_type.is_dir() {
        "dir"
    } else if entry_type.is_symlink() {
        "symlink"
    } else if entry_type.is_hard_link() {
        "hardlink"
    } else if entry_type.is_character_special() {
        "char"
    } else if entry_type.is_block_special() {
        "block"
    } else if entry_type.is_fifo() {
        "fifo"
    } else {
        "reg"
    }
}

fn stream_payload<R: Read + ?Sized>(
    encoded: &mut GzEncoder<&mut Vec<u8>>,
    digest: &mut Sha256,
    uncompressed: &mut Uncompressed,
    payload: &mut R,
) -> Result<()> {
    let mut hashed = HashingRead {
        inner: payload,
        digest,
        uncompressed,
    };
    io::copy(&mut hashed, encoded).map_err(KociError::IoError)?;

    Ok(())
}

fn pad_member(
    encoded: &mut GzEncoder<&mut Vec<u8>>,
    uncompressed: &mut Uncompressed,
    size: u64,
) -> Result<()> {
    let aligned = size
        .checked_next_multiple_of(512)
        .ok_or_else(|| KociError::PushError("member padding overflows".to_owned()))?;
    let missing = aligned.saturating_sub(size);
    let missing = usize::try_from(missing).unwrap_or(0);

    let zeros = vec![0_u8; missing];
    uncompressed.update(&zeros);
    io::copy(&mut zeros.as_slice(), encoded).map_err(KociError::IoError)?;

    Ok(())
}

struct HashingRead<'a, R: Read> {
    inner: R,
    digest: &'a mut Sha256,
    uncompressed: &'a mut Uncompressed,
}

impl<R: Read> Read for HashingRead<'_, R> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let copied = self.inner.read(buf)?;
        let chunk = buf.get(..copied).unwrap_or(buf);
        self.digest.update(chunk);
        self.uncompressed.update(chunk);

        Ok(copied)
    }
}
