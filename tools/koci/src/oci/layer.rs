//! OCI layer download, digest verification, and archive extraction.

use std::fs;
use std::io::{Cursor, Read};
use std::path::{Component, Path, PathBuf};

use flate2::read::GzDecoder;
use tar::Archive;

use crate::error::{KociError, Result};
use crate::image::ImageReference;
use crate::oci::http::{HttpClient, collect_body, get};
use crate::oci::verify::verify_blob_digest;

const OCI_LAYER_TAR: &str = "application/vnd.oci.image.layer.v1.tar";
const OCI_LAYER_TAR_GZIP: &str = "application/vnd.oci.image.layer.v1.tar+gzip";
const DOCKER_LAYER_TAR_GZIP: &str = "application/vnd.docker.image.rootfs.diff.tar.gzip";

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

/// Extract a gzip-compressed tar archive from an OCI layer blob.
pub(crate) fn extract_archive(bytes: &[u8], media_type: Option<&str>, dest: &Path) -> Result<()> {
    match media_type.unwrap_or(OCI_LAYER_TAR_GZIP) {
        OCI_LAYER_TAR_GZIP | DOCKER_LAYER_TAR_GZIP => {
            let decoder = GzDecoder::new(Cursor::new(bytes));
            extract_tar(decoder, dest)
        }
        OCI_LAYER_TAR => extract_tar(Cursor::new(bytes), dest),
        other => Err(KociError::UnsupportedLayerMediaType(other.to_string())),
    }
}

fn extract_tar<R: Read>(reader: R, dest: &Path) -> Result<()> {
    let mut archive = Archive::new(reader);

    for entry in archive.entries().map_err(layer_extract_error)? {
        let mut entry = entry.map_err(layer_extract_error)?;
        let header = entry.header().clone();
        let entry_type = header.entry_type();
        let relative_path =
            normalize_entry_path(entry.path().map_err(layer_extract_error)?.as_ref())?;
        let Some(relative_path) = relative_path else {
            continue;
        };

        if let Some(whiteout) = whiteout_target(dest, &relative_path)? {
            apply_whiteout(&whiteout, dest)?;
            continue;
        }

        let output_path = dest.join(&relative_path);
        ensure_within_root(dest, &output_path)?;

        if entry_type.is_dir() {
            fs::create_dir_all(&output_path)?;
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

        if let Some(parent) = output_path.parent() {
            fs::create_dir_all(parent)?;
        }

        let mut file = fs::File::create(&output_path)?;
        std::io::copy(&mut entry, &mut file)?;
    }

    Ok(())
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
            Component::Prefix(_) => {
                return Err(KociError::LayerExtractionError(format!(
                    "OCI layer entry uses unsupported path prefix: {}",
                    path.display()
                )));
            }
        }
    }

    if normalized.as_os_str().is_empty() {
        Ok(None)
    } else {
        Ok(Some(normalized))
    }
}

fn whiteout_target(dest: &Path, path: &Path) -> Result<Option<PathBuf>> {
    let Some(file_name) = path.file_name().and_then(|name| name.to_str()) else {
        return Ok(None);
    };

    if file_name == ".wh..wh..opq" {
        let parent = path.parent().unwrap_or_else(|| Path::new(""));
        let target = dest.join(parent);
        ensure_within_root(dest, &target)?;
        return Ok(Some(target));
    }

    let Some(stripped) = file_name.strip_prefix(".wh.") else {
        return Ok(None);
    };

    let parent = path.parent().unwrap_or_else(|| Path::new(""));
    let target = dest.join(parent).join(stripped);
    ensure_within_root(dest, &target)?;
    Ok(Some(target))
}

fn apply_whiteout(target: &Path, dest: &Path) -> Result<()> {
    ensure_within_root(dest, target)?;

    if !target.exists() {
        return Ok(());
    }

    let metadata = fs::symlink_metadata(target)?;
    if metadata.is_dir() {
        fs::remove_dir_all(target)?;
    } else {
        fs::remove_file(target)?;
    }

    Ok(())
}

fn ensure_within_root(root: &Path, candidate: &Path) -> Result<()> {
    if candidate.starts_with(root) {
        Ok(())
    } else {
        Err(KociError::LayerExtractionError(format!(
            "OCI layer entry escapes extraction root: {}",
            candidate.display()
        )))
    }
}

fn layer_extract_error(error: std::io::Error) -> KociError {
    KociError::LayerExtractionError(format!("Failed to extract tar: {error}"))
}

#[cfg(test)]
mod tests {
    use std::error::Error;

    use flate2::Compression;
    use flate2::write::GzEncoder;
    use tar::{Builder, EntryType, Header};
    use tempfile::TempDir;

    use super::*;

    fn archive_bytes(entries: &[(&str, &[u8])]) -> std::result::Result<Vec<u8>, Box<dyn Error>> {
        let encoder = GzEncoder::new(Vec::new(), Compression::default());
        let mut archive = Builder::new(encoder);

        for (path, bytes) in entries {
            let mut header = Header::new_gnu();
            header.set_size(bytes.len() as u64);
            header.set_mode(0o644);
            header.set_cksum();
            archive.append_data(&mut header, path, *bytes)?;
        }

        let encoder = archive.into_inner()?;
        Ok(encoder.finish()?)
    }

    fn archive_with_entry(
        path: &str,
        entry_type: EntryType,
        bytes: &[u8],
        link_name: Option<&str>,
    ) -> std::result::Result<Vec<u8>, Box<dyn Error>> {
        let encoder = GzEncoder::new(Vec::new(), Compression::default());
        let mut archive = Builder::new(encoder);
        let mut header = Header::new_gnu();

        header.set_entry_type(entry_type);
        header.set_size(bytes.len() as u64);
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
    ) -> std::result::Result<Vec<u8>, Box<dyn Error>> {
        let mut archive = Builder::new(Vec::new());

        for (path, bytes) in entries {
            let mut header = Header::new_gnu();
            header.set_size(bytes.len() as u64);
            header.set_mode(0o644);
            header.set_cksum();
            archive.append_data(&mut header, path, *bytes)?;
        }

        Ok(archive.into_inner()?)
    }

    #[test]
    fn extract_archive_rejects_invalid_gzip_data() {
        // ARRANGE
        let output = TempDir::new().expect("create temp dir");

        // ACT
        let result = extract_archive(b"not a gzip archive", None, output.path());

        // ASSERT
        assert!(matches!(result, Err(KociError::LayerExtractionError(_))));
    }

    #[test]
    fn extract_archive_applies_whiteout_file_removal() {
        // ARRANGE
        let output = TempDir::new().expect("create temp dir");
        fs::create_dir_all(output.path().join("etc")).expect("create etc dir");
        fs::write(output.path().join("etc/obsolete"), b"stale").expect("write stale file");
        let layer = archive_bytes(&[("etc/.wh.obsolete", b"")]).expect("build layer archive");

        // ACT
        extract_archive(&layer, None, output.path()).expect("extract layer");

        // ASSERT
        assert!(!output.path().join("etc/obsolete").exists());
    }

    #[test]
    fn extract_archive_applies_opaque_whiteout_directory_removal() {
        // ARRANGE
        let output = TempDir::new().expect("create temp dir");
        fs::create_dir_all(output.path().join("etc/conf.d")).expect("create conf dir");
        fs::write(output.path().join("etc/conf.d/old"), b"stale").expect("write stale file");
        let layer = archive_bytes(&[("etc/conf.d/.wh..wh..opq", b"")]).expect("build layer");

        // ACT
        extract_archive(&layer, None, output.path()).expect("extract layer");

        // ASSERT
        assert!(!output.path().join("etc/conf.d").exists());
    }

    #[test]
    fn extract_archive_supports_raw_tar_layers() {
        // ARRANGE
        let output = TempDir::new().expect("create temp dir");
        let layer = raw_archive_bytes(&[("etc/raw", b"raw tar layer\n")]).expect("build layer");

        // ACT
        extract_archive(&layer, Some(OCI_LAYER_TAR), output.path()).expect("extract layer");

        // ASSERT
        assert_eq!(
            fs::read_to_string(output.path().join("etc/raw")).expect("read extracted file"),
            "raw tar layer\n"
        );
    }

    #[test]
    fn extract_archive_creates_directory_entries() {
        // ARRANGE
        let output = TempDir::new().expect("create temp dir");
        let layer = archive_with_entry("etc/nested/", EntryType::dir(), b"", None)
            .expect("build layer archive");

        // ACT
        extract_archive(&layer, None, output.path()).expect("extract layer");

        // ASSERT
        assert!(output.path().join("etc/nested").is_dir());
    }

    #[test]
    fn extract_archive_skips_current_directory_entry() {
        // ARRANGE
        let output = TempDir::new().expect("create temp dir");
        let layer = archive_bytes(&[("./", b"")]).expect("build layer archive");

        // ACT
        extract_archive(&layer, None, output.path()).expect("extract layer");

        // ASSERT
        assert!(
            fs::read_dir(output.path())
                .expect("read output dir")
                .next()
                .is_none()
        );
    }

    #[test]
    fn extract_archive_ignores_whiteout_when_target_is_missing() {
        // ARRANGE
        let output = TempDir::new().expect("create temp dir");
        let layer = archive_bytes(&[("etc/.wh.missing", b"")]).expect("build layer archive");

        // ACT
        extract_archive(&layer, None, output.path()).expect("extract layer");

        // ASSERT
        assert!(
            fs::read_dir(output.path())
                .expect("read output dir")
                .next()
                .is_none()
        );
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
        let output = TempDir::new().expect("create temp dir");
        let layer = archive_with_entry("etc/link", EntryType::symlink(), b"", Some("target"))
            .expect("build layer archive");

        // ACT
        let error = extract_archive(&layer, None, output.path()).expect_err("extract should fail");

        // ASSERT
        assert!(matches!(error, KociError::LayerExtractionError(_)));
    }

    #[test]
    fn extract_archive_rejects_hard_link_entries() {
        // ARRANGE
        let output = TempDir::new().expect("create temp dir");
        let layer = archive_with_entry("etc/link", EntryType::hard_link(), b"", Some("target"))
            .expect("build layer archive");

        // ACT
        let error = extract_archive(&layer, None, output.path()).expect_err("extract should fail");

        // ASSERT
        assert!(matches!(error, KociError::LayerExtractionError(_)));
    }

    #[test]
    fn extract_archive_rejects_fifo_entries() {
        // ARRANGE
        let output = TempDir::new().expect("create temp dir");
        let layer = archive_with_entry("etc/fifo", EntryType::fifo(), b"", None)
            .expect("build layer archive");

        // ACT
        let error = extract_archive(&layer, None, output.path()).expect_err("extract should fail");

        // ASSERT
        assert!(matches!(error, KociError::LayerExtractionError(_)));
    }

    #[test]
    fn extract_archive_rejects_unsupported_media_type() {
        // ARRANGE
        let output = TempDir::new().expect("create temp dir");

        // ACT
        let error = extract_archive(b"irrelevant", Some("application/test"), output.path())
            .expect_err("extract should fail");

        // ASSERT
        assert!(matches!(error, KociError::UnsupportedLayerMediaType(_)));
    }

    #[test]
    fn whiteout_target_returns_none_for_non_whiteout_path() {
        // ARRANGE
        let output = TempDir::new().expect("create temp dir");

        // ACT
        let target =
            whiteout_target(output.path(), Path::new("etc/file")).expect("resolve whiteout target");

        // ASSERT
        assert!(target.is_none());
    }

    #[test]
    fn apply_whiteout_rejects_target_outside_root() {
        // ARRANGE
        let root = TempDir::new().expect("create temp dir");
        let outside = TempDir::new().expect("create second temp dir");

        // ACT
        let error = apply_whiteout(outside.path(), root.path()).expect_err("whiteout should fail");

        // ASSERT
        assert!(matches!(error, KociError::LayerExtractionError(_)));
    }

    #[test]
    fn ensure_within_root_rejects_path_outside_root() {
        // ARRANGE
        let root = TempDir::new().expect("create temp dir");
        let outside = TempDir::new().expect("create second temp dir");

        // ACT
        let error =
            ensure_within_root(root.path(), outside.path()).expect_err("path check should fail");

        // ASSERT
        assert!(matches!(error, KociError::LayerExtractionError(_)));
    }
}
