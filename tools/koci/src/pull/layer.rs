//! OCI layer download, digest verification, and in-memory application.

use std::io::{Cursor, Read};
use std::path::{Component, Path, PathBuf};

use flate2::read::GzDecoder;
use tar::Archive;

use crate::digest::verify_blob_digest;
use crate::error::{KociError, Result};
use crate::image::ImageReference;
use crate::pulled::PulledImage;
use crate::registry::http::{HttpClient, collect_body, get};

const OCI_LAYER_TAR: &str = "application/vnd.oci.image.layer.v1.tar";
const OCI_LAYER_TAR_GZIP: &str = "application/vnd.oci.image.layer.v1.tar+gzip";
const DOCKER_LAYER_TAR_GZIP: &str = "application/vnd.docker.image.rootfs.diff.tar.gzip";
const DEFAULT_FILE_MODE: u32 = 0o644;
const DEFAULT_DIR_MODE: u32 = 0o755;

/// Download a blob from the registry, verify its SHA-256 digest, and return the raw bytes.
pub(crate) async fn download_blob(
    client: &HttpClient,
    image_ref: &ImageReference,
    digest: &str,
    token: Option<&str>,
) -> Result<Vec<u8>> {
    let blob_url = format!(
        "{}://{}/v2/{}/blobs/{}",
        image_ref.scheme(),
        image_ref.registry,
        image_ref.name,
        digest
    );

    let resp = get(client, &blob_url, token, &[]).await?;
    let bytes = collect_body(resp).await?.to_vec();
    verify_blob_digest(&bytes, digest)?;
    Ok(bytes)
}

/// Apply an OCI layer blob onto a merged in-memory image.
pub(crate) fn extract_archive(
    bytes: &[u8],
    media_type: Option<&str>,
    image: PulledImage,
) -> Result<PulledImage> {
    match media_type.unwrap_or(OCI_LAYER_TAR_GZIP) {
        OCI_LAYER_TAR_GZIP | DOCKER_LAYER_TAR_GZIP => {
            let decoder = GzDecoder::new(Cursor::new(bytes));
            extract_tar(decoder, image)
        }
        OCI_LAYER_TAR => extract_tar(Cursor::new(bytes), image),
        other => Err(KociError::UnsupportedLayerMediaType(other.to_owned())),
    }
}

fn extract_tar<R: Read>(reader: R, mut image: PulledImage) -> Result<PulledImage> {
    let mut archive = Archive::new(reader);

    for entry in archive
        .entries()
        .map_err(|error| layer_extract_error(&error))?
    {
        let mut entry = entry.map_err(|error| layer_extract_error(&error))?;
        let header = entry.header().clone();
        let entry_type = header.entry_type();
        let relative_path = normalize_entry_path(
            entry
                .path()
                .map_err(|error| layer_extract_error(&error))?
                .as_ref(),
        )?;
        let Some(relative_path) = relative_path else {
            continue;
        };

        if let Some(whiteout) = whiteout_target(&relative_path) {
            image.remove_path(&whiteout);
            continue;
        }

        if entry_type.is_dir() {
            image.insert_dir(&relative_path, DEFAULT_DIR_MODE);
            continue;
        }

        if entry_type.is_symlink() {
            return Err(KociError::LayerExtractionError(format!(
                "Unsupported symlink entry in OCI layer: {}",
                relative_path.display()
            )));
        }

        if entry_type.is_hard_link() {
            return Err(KociError::LayerExtractionError(format!(
                "Unsupported hard link entry in OCI layer: {}",
                relative_path.display()
            )));
        }

        if !entry_type.is_file() {
            return Err(KociError::LayerExtractionError(format!(
                "Unsupported OCI layer entry type for {}",
                relative_path.display()
            )));
        }

        let mut data = Vec::new();
        entry.read_to_end(&mut data).map_err(|source| {
            KociError::LayerExtractionError(format!("Failed to read file data: {source}"))
        })?;
        image.insert_file(&relative_path, DEFAULT_FILE_MODE, data);
    }

    Ok(image)
}

fn normalize_entry_path(path: &Path) -> Result<Option<PathBuf>> {
    let mut normalized = PathBuf::new();

    for component in path.components() {
        match component {
            Component::Normal(part) => normalized.push(part),
            Component::CurDir | Component::RootDir => {}
            Component::ParentDir => {
                return Err(KociError::LayerExtractionError(format!(
                    "OCI layer entry escapes extraction root: {}",
                    path.display()
                )));
            }
            Component::Prefix(prefix) => {
                #[cfg(windows)]
                {
                    let _ = prefix;
                    return Err(KociError::LayerExtractionError(format!(
                        "OCI layer entry uses unsupported path prefix: {}",
                        path.display()
                    )));
                }

                #[cfg(not(windows))]
                normalized.push(prefix.as_os_str());
            }
        }
    }

    if normalized.as_os_str().is_empty() {
        Ok(None)
    } else {
        Ok(Some(normalized))
    }
}

fn whiteout_target(path: &Path) -> Option<PathBuf> {
    let file_name = path.file_name().and_then(|name| name.to_str())?;

    if file_name == ".wh..wh..opq" {
        return Some(path.parent().unwrap_or_else(|| Path::new("")).to_path_buf());
    }

    let stripped = file_name.strip_prefix(".wh.")?;

    let parent = path.parent().unwrap_or_else(|| Path::new(""));
    Some(parent.join(stripped))
}

fn layer_extract_error(error: &std::io::Error) -> KociError {
    KociError::LayerExtractionError(format!("Failed to extract tar: {error}"))
}

#[cfg(test)]
mod tests {
    use core::error::Error;

    use flate2::Compression;
    use flate2::write::GzEncoder;
    use tar::{Builder, EntryType, Header};

    use super::*;

    fn archive_bytes(entries: &[(&str, &[u8])]) -> core::result::Result<Vec<u8>, Box<dyn Error>> {
        let encoder = GzEncoder::new(Vec::new(), Compression::default());
        let mut archive = Builder::new(encoder);

        for &(path, bytes) in entries {
            let mut header = Header::new_gnu();
            header.set_size(u64::try_from(bytes.len())?);
            header.set_mode(0o644);
            header.set_cksum();
            archive.append_data(&mut header, path, bytes)?;
        }

        let encoder = archive.into_inner()?;
        Ok(encoder.finish()?)
    }

    fn archive_with_entry(
        path: &str,
        entry_type: EntryType,
        bytes: &[u8],
        link_name: Option<&str>,
    ) -> core::result::Result<Vec<u8>, Box<dyn Error>> {
        let encoder = GzEncoder::new(Vec::new(), Compression::default());
        let mut archive = Builder::new(encoder);
        let mut header = Header::new_gnu();

        header.set_entry_type(entry_type);
        header.set_size(u64::try_from(bytes.len())?);
        header.set_mode(0o644);
        if let Some(link_name) = link_name {
            header.set_link_name(link_name)?;
        }
        header.set_cksum();
        archive.append_data(&mut header, path, bytes)?;

        let encoder = archive.into_inner()?;
        Ok(encoder.finish()?)
    }

    fn raw_archive_bytes(
        entries: &[(&str, &[u8])],
    ) -> core::result::Result<Vec<u8>, Box<dyn Error>> {
        let mut archive = Builder::new(Vec::new());

        for &(path, bytes) in entries {
            let mut header = Header::new_gnu();
            header.set_size(u64::try_from(bytes.len())?);
            header.set_mode(0o644);
            header.set_cksum();
            archive.append_data(&mut header, path, bytes)?;
        }

        Ok(archive.into_inner()?)
    }

    #[test]
    fn normalize_entry_path_returns_none_for_current_directory() {
        // ARRANGE
        let path = Path::new("./");

        // ACT
        let normalized = normalize_entry_path(path).expect("normalize path");

        // ASSERT
        assert!(normalized.is_none());
    }

    #[test]
    fn extract_archive_writes_root_level_file() {
        // ARRANGE
        let image = PulledImage::new();
        let layer = archive_bytes(&[("root.txt", b"hello\n")]).expect("build layer archive");

        // ACT
        let image = extract_archive(&layer, None, image).expect("extract layer");

        // ASSERT
        let file = image
            .file(Path::new("root.txt"))
            .expect("file lookup")
            .expect("missing root file");
        let mut reader = file.open().expect("open file");
        let mut contents = String::new();
        reader.read_to_string(&mut contents).expect("read file");
        assert_eq!(contents, "hello\n");
    }

    #[test]
    fn extract_archive_applies_whiteout_file_removal() {
        // ARRANGE
        let image = {
            let mut image = PulledImage::new();
            image.insert_file(
                Path::new("etc/obsolete"),
                DEFAULT_FILE_MODE,
                b"stale".to_vec(),
            );
            image
        };
        let layer = archive_bytes(&[("etc/.wh.obsolete", b"")]).expect("build layer archive");

        // ACT
        let image = extract_archive(&layer, None, image).expect("extract layer");

        // ASSERT
        assert!(
            image
                .file(Path::new("etc/obsolete"))
                .expect("file lookup")
                .is_none()
        );
    }

    #[test]
    fn extract_archive_applies_opaque_whiteout_directory_removal() {
        // ARRANGE
        let image = {
            let mut image = PulledImage::new();
            image.insert_file(
                Path::new("etc/conf.d/old"),
                DEFAULT_FILE_MODE,
                b"stale".to_vec(),
            );
            image
        };
        let layer = archive_bytes(&[("etc/conf.d/.wh..wh..opq", b"")]).expect("build layer");

        // ACT
        let image = extract_archive(&layer, None, image).expect("extract layer");

        // ASSERT
        assert!(
            image
                .file(Path::new("etc/conf.d/old"))
                .expect("file lookup")
                .is_none()
        );
    }

    #[test]
    fn extract_archive_supports_raw_tar_layers() {
        // ARRANGE
        let image = PulledImage::new();
        let layer = raw_archive_bytes(&[("etc/raw", b"raw tar layer\n")]).expect("build layer");

        // ACT
        let image = extract_archive(&layer, Some(OCI_LAYER_TAR), image).expect("extract layer");

        // ASSERT
        let file = image
            .file(Path::new("etc/raw"))
            .expect("file lookup")
            .expect("missing raw file");
        let mut reader = file.open().expect("open file");
        let mut contents = String::new();
        reader.read_to_string(&mut contents).expect("read file");
        assert_eq!(contents, "raw tar layer\n");
    }

    #[test]
    fn extract_archive_creates_directory_entries() {
        // ARRANGE
        let image = PulledImage::new();
        let layer = archive_with_entry("etc/nested/", EntryType::dir(), b"", None)
            .expect("build layer archive");

        // ACT
        let image = extract_archive(&layer, None, image).expect("extract layer");

        // ASSERT
        let paths: Vec<PathBuf> = image
            .entries()
            .expect("entries")
            .iter()
            .map(|entry| entry.path().to_path_buf())
            .collect();
        assert!(paths.contains(&PathBuf::from("etc/nested")));
    }

    #[test]
    fn extract_archive_skips_current_directory_entry() {
        // ARRANGE
        let image = PulledImage::new();
        let layer = archive_bytes(&[("./", b"")]).expect("build layer archive");

        // ACT
        let image = extract_archive(&layer, None, image).expect("extract layer");

        // ASSERT
        assert!(image.entries().expect("entries").is_empty());
    }

    #[test]
    fn extract_archive_ignores_whiteout_when_target_is_missing() {
        // ARRANGE
        let image = PulledImage::new();
        let layer = archive_bytes(&[("etc/.wh.missing", b"")]).expect("build layer archive");

        // ACT
        let image = extract_archive(&layer, None, image).expect("extract layer");

        // ASSERT
        assert!(image.entries().expect("entries").is_empty());
    }

    #[test]
    fn extract_archive_rejects_parent_traversal() {
        // ACT
        let error =
            normalize_entry_path(Path::new("../escape")).expect_err("normalize should fail");

        // ASSERT
        assert!(matches!(error, KociError::LayerExtractionError(_)));
    }

    #[test]
    fn extract_archive_rejects_symlink_entries() {
        // ARRANGE
        let layer = archive_with_entry("etc/link", EntryType::symlink(), b"", Some("target"))
            .expect("build layer archive");

        // ACT
        let error =
            extract_archive(&layer, None, PulledImage::new()).expect_err("extract should fail");

        // ASSERT
        assert!(matches!(error, KociError::LayerExtractionError(_)));
    }

    #[test]
    fn extract_archive_rejects_hard_link_entries() {
        // ARRANGE
        let layer = archive_with_entry("etc/link", EntryType::hard_link(), b"", Some("target"))
            .expect("build layer archive");

        // ACT
        let error =
            extract_archive(&layer, None, PulledImage::new()).expect_err("extract should fail");

        // ASSERT
        assert!(matches!(error, KociError::LayerExtractionError(_)));
    }

    #[test]
    fn extract_archive_rejects_fifo_entries() {
        // ARRANGE
        let layer = archive_with_entry("etc/fifo", EntryType::fifo(), b"", None)
            .expect("build layer archive");

        // ACT
        let error =
            extract_archive(&layer, None, PulledImage::new()).expect_err("extract should fail");

        // ASSERT
        assert!(matches!(error, KociError::LayerExtractionError(_)));
    }

    #[test]
    fn extract_archive_rejects_unsupported_media_type() {
        // ACT
        let error = extract_archive(b"irrelevant", Some("application/test"), PulledImage::new())
            .expect_err("extract should fail");

        // ASSERT
        assert!(matches!(error, KociError::UnsupportedLayerMediaType(_)));
    }

    #[test]
    fn whiteout_target_returns_none_for_non_whiteout_path() {
        // ACT
        let target = whiteout_target(Path::new("etc/file"));

        // ASSERT
        assert!(target.is_none());
    }
}
