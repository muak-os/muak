//! TOC layout pieces.

use core::hash::BuildHasher;
use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::error::Result;

/// Name of the TOC tar entry inside the layer blob.
pub const TOC_NAME: &str = "stargz.index.json";

/// Byte length of the footer appended after the final gzip member.
pub const FOOTER_SIZE: usize = 51;

/// Layer descriptor annotation carrying the digest of the TOC JSON payload.
pub const TOC_DIGEST_ANNOTATION: &str = "containerd.io/snapshot/stargz/toc.digest";

/// Layer descriptor annotation carrying the layer's uncompressed byte count.
pub const UNCOMPRESSED_SIZE_ANNOTATION: &str = "io.containers.estargz.uncompressed-size";

/// Returns whether `annotations` mark a layer as eStargz.
#[must_use]
pub fn is_estargz<S: BuildHasher>(annotations: &HashMap<String, String, S>) -> bool {
    annotations.contains_key(TOC_DIGEST_ANNOTATION)
}

/// gzip header magic and flags of the footer member.
const FOOTER_HEAD: [u8; 10] = [0x1f, 0x8b, 0x08, 0x04, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00];

/// Deflate framing of an empty stored block, padding the footer member.
const EMPTY_BLOCK: [u8; 5] = [0x01, 0x00, 0x00, 0xff, 0xff];

/// Hex digits used by the footer's Extra field.
const HEX: &[u8] = b"0123456789abcdef";

/// Serializes the JSON table of contents.
#[derive(Debug, Serialize, Deserialize)]
pub struct Toc {
    /// TOC schema version, always 1.
    version: u64,
    /// Metadata of every tar entry and chunk except the TOC itself.
    entries: Vec<Record>,
}

impl Toc {
    /// Consumes the TOC, returning its entries in blob order.
    #[must_use]
    pub(crate) fn into_entries(self) -> Vec<Record> {
        self.entries
    }

    /// Serializes `entries` into the TOC JSON payload and its sha256 digest.
    ///
    /// # Errors
    ///
    /// Returns an error when TOC serialization fails.
    pub fn encode(entries: &[Record]) -> Result<(Vec<u8>, String)> {
        let payload = serde_json::to_vec(&Self {
            version: 1,
            entries: entries.to_vec(),
        })
        .map_err(|error| crate::error::KociError::PushError(format!("serialize TOC: {error}")))?;
        let digest = format!("sha256:{}", oci::digest::sha256_hex(&payload));

        Ok((payload, digest))
    }
}

/// One tar entry or chunk of the layer, recorded inside the TOC.
#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct Record {
    /// Full tar entry path, directories without their trailing slash.
    pub name: String,
    /// One of `dir`, `reg`, `chunk`, `symlink`, `hardlink`, `char`, `block`, `fifo`.
    #[serde(rename = "type")]
    pub kind: String,
    /// Uncompressed regular-file size, omitted when zero.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub size: Option<u64>,
    /// Permission bits, omitted when zero.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mode: Option<u64>,
    /// Compressed blob offset of the entry's gzip member.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub offset: Option<u64>,
    /// SHA-256 of the uncompressed regular-file payload.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub digest: Option<String>,
    /// SHA-256 of this chunk of the payload, required for non-empty files.
    #[serde(rename = "chunkDigest", skip_serializing_if = "Option::is_none")]
    pub chunk_digest: Option<String>,
}

/// Encodes the 51-byte footer member pointing at `offset`.
///
/// # Panics
///
/// Never: the formatter always fills exactly 16 hex digits.
#[must_use]
pub fn footer(offset: u64) -> [u8; FOOTER_SIZE] {
    let mut footer = [0_u8; FOOTER_SIZE];
    footer[..10].copy_from_slice(&FOOTER_HEAD);
    footer[10..12].copy_from_slice(&26_u16.to_le_bytes());
    footer[12] = b'S';
    footer[13] = b'G';
    footer[14..16].copy_from_slice(&22_u16.to_le_bytes());
    footer[16..32].copy_from_slice(format!("{offset:016x}").as_bytes());
    footer[32..38].copy_from_slice(b"STARGZ");
    footer[38..43].copy_from_slice(&EMPTY_BLOCK);

    footer
}

/// Parses the TOC compressed offset out of footer bytes from a blob tail.
#[must_use]
pub fn parse_footer(footer: &[u8]) -> Option<u64> {
    if footer.len() != FOOTER_SIZE || !framed(footer) {
        return None;
    }

    let field = footer.get(16..38)?;
    let (digits, magic) = field.split_at(16);
    if magic != *b"STARGZ" || digits.iter().any(|digit| !HEX.contains(digit)) {
        return None;
    }

    digits.iter().try_fold(0_u64, |offset, digit| {
        let weight =
            u64::try_from(HEX.iter().position(|hex| hex == digit).unwrap_or_default()).ok()?;
        offset.checked_mul(16)?.checked_add(weight)
    })
}

fn framed(footer: &[u8]) -> bool {
    if footer.len() < 16 {
        return false;
    }

    let flags = footer.get(12) == Some(&b'S')
        && footer.get(13) == Some(&b'G')
        && footer
            .get(14..16)
            .is_some_and(|frame| frame.iter().eq(22_u16.to_le_bytes().iter()))
        && footer
            .get(10..12)
            .is_some_and(|frame| frame.iter().eq(26_u16.to_le_bytes().iter()));

    footer
        .get(..10)
        .is_some_and(|head| head.iter().eq(FOOTER_HEAD.iter()))
        && flags
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn is_estargz_reads_the_toc_digest_annotation() {
        // ARRANGE
        let mut annotated = HashMap::new();
        annotated.insert(TOC_DIGEST_ANNOTATION.to_owned(), "sha256:x".to_owned());

        // ACT / ASSERT
        assert!(is_estargz(&annotated));
        assert!(!is_estargz(&HashMap::new()));
    }
}
