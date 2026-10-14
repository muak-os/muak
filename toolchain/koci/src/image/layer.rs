//! Turns a source layer into a deterministic eStargz blob or passes it through.

use std::collections::HashMap;

use flate2::read::MultiGzDecoder;
use oci::digest::Verifier;
use oci::media::{
    DOCKER_LAYER_GZIP_MEDIA_TYPE, DOCKER_LAYER_MEDIA_TYPE, OCI_LAYER_GZIP_MEDIA_TYPE,
    OCI_LAYER_MEDIA_TYPE,
};
use oci::model::Descriptor;

use crate::error::{KociError, Result};
use crate::stargz::toc::{self, TOC_DIGEST_ANNOTATION, UNCOMPRESSED_SIZE_ANNOTATION};
use crate::stargz::{self, LayerBlob};

/// One layer prepared for upload.
#[derive(Debug)]
pub(crate) struct Prepared {
    /// Descriptor to place in the output manifest.
    pub(crate) descriptor: Descriptor,
    /// Replacement diff id, or `None` to reuse the source config's value.
    pub(crate) diff_id: Option<String>,
    /// Blob bytes to upload.
    pub(crate) bytes: Vec<u8>,
}

/// Verifies and, when needed, rewraps one source layer's raw bytes.
///
/// # Errors
///
/// Returns an error when the blob fails verification or cannot be decoded and repacked.
pub(crate) fn prepare(raw: Vec<u8>, source: &Descriptor) -> Result<Prepared> {
    verify(&source.digest, &raw)?;

    if source.annotations.as_ref().is_some_and(toc::is_estargz) {
        return Ok(Prepared {
            descriptor: source.clone(),
            diff_id: None,
            bytes: raw,
        });
    }

    rewrap(&raw, source.media_type.as_deref())
}

fn rewrap(raw: &[u8], media_type: Option<&str>) -> Result<Prepared> {
    let blob = match media_type {
        Some(OCI_LAYER_GZIP_MEDIA_TYPE | DOCKER_LAYER_GZIP_MEDIA_TYPE) => {
            stargz::ingest::from_tar(MultiGzDecoder::new(raw))?
        }
        Some(OCI_LAYER_MEDIA_TYPE | DOCKER_LAYER_MEDIA_TYPE) => stargz::ingest::from_tar(raw)?,
        Some(other) => return Err(KociError::UnsupportedLayerMediaType(other.to_owned())),
        None => {
            return Err(KociError::PushError(
                "layer descriptor has no media type".to_owned(),
            ));
        }
    };

    let LayerBlob {
        bytes,
        digest,
        size,
        uncompressed,
        uncompressed_size,
        toc_digest,
    } = blob;
    let mut annotations = HashMap::new();
    annotations.insert(TOC_DIGEST_ANNOTATION.to_owned(), toc_digest);
    annotations.insert(
        UNCOMPRESSED_SIZE_ANNOTATION.to_owned(),
        uncompressed_size.to_string(),
    );

    Ok(Prepared {
        descriptor: Descriptor {
            media_type: Some(OCI_LAYER_GZIP_MEDIA_TYPE.to_owned()),
            digest,
            size,
            platform: None,
            annotations: Some(annotations),
        },
        diff_id: Some(uncompressed),
        bytes,
    })
}

fn verify(digest: &str, bytes: &[u8]) -> Result<()> {
    let mut verifier = Verifier::new(digest)?;
    verifier.update(bytes);

    Ok(verifier.verify()?)
}

#[cfg(test)]
mod tests {
    use flate2::read::MultiGzDecoder;
    use tar::{Archive, Header};

    use super::*;

    fn tar_with(entries: &[(&str, &[u8])]) -> Vec<u8> {
        let mut builder = tar::Builder::new(Vec::new());
        for (path, data) in entries.iter().copied() {
            let mut header = Header::new_gnu();
            header.set_size(u64::try_from(data.len()).expect("entry size"));
            header.set_mode(0o644);
            header.set_mtime(42);
            builder
                .append_data(&mut header, path, data)
                .expect("append entry");
        }

        builder.into_inner().expect("finish tar")
    }

    #[test]
    fn rewrap_preserves_long_paths_and_is_deterministic() {
        // ARRANGE
        let long = format!("usr/lib/{}libfoo.so", "nested/".repeat(30));
        let tar = tar_with(&[(long.as_str(), b"ELF")]);

        // ACT
        let first = rewrap(&tar, Some(OCI_LAYER_MEDIA_TYPE)).expect("rewrap");
        let second = rewrap(&tar, Some(OCI_LAYER_MEDIA_TYPE)).expect("rewrap again");

        // ASSERT
        assert_eq!(first.descriptor.digest, second.descriptor.digest);
        let names: Vec<String> = Archive::new(MultiGzDecoder::new(first.bytes.as_slice()))
            .entries()
            .expect("entries")
            .filter_map(std::result::Result::ok)
            .map(|entry| String::from_utf8_lossy(entry.path_bytes().as_ref()).into_owned())
            .collect();
        assert!(names.contains(&long), "missing {long}: {names:?}");
        assert!(first.diff_id.is_some(), "rewrap must compute a diff id");
    }

    #[test]
    fn rewrap_rejects_unsupported_and_missing_media_types() {
        // ARRANGE / ACT / ASSERT
        assert!(matches!(
            rewrap(b"", Some("application/x-custom")),
            Err(KociError::UnsupportedLayerMediaType(_))
        ));
        assert!(matches!(rewrap(b"", None), Err(KociError::PushError(_))));
    }

    #[test]
    fn rewrap_diff_id_matches_the_decompressed_layer() {
        // ARRANGE
        use std::io::Read as _;

        let tar = tar_with(&[("etc/hostname", b"muak")]);

        // ACT
        let prepared = rewrap(&tar, Some(OCI_LAYER_MEDIA_TYPE)).expect("rewrap");
        let mut decompressed = Vec::new();
        MultiGzDecoder::new(prepared.bytes.as_slice())
            .read_to_end(&mut decompressed)
            .expect("gunzip");

        // ASSERT
        let size = prepared
            .descriptor
            .annotations
            .as_ref()
            .and_then(|annotations| annotations.get(UNCOMPRESSED_SIZE_ANNOTATION))
            .and_then(|value| value.parse::<u64>().ok());
        assert_eq!(
            size,
            Some(u64::try_from(decompressed.len()).expect("length"))
        );
        let expected = format!("sha256:{}", oci::digest::sha256_hex(&decompressed));
        assert_eq!(prepared.diff_id.as_deref(), Some(expected.as_str()));
    }
}
