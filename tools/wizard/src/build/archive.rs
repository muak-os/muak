//! Initramfs archive building.

use std::collections::HashMap;
use std::io::{Read as _, Write};
use std::path::Path;

use erofs::tree::TreeEntry;
use koci::pulled::{PulledEntry, PulledImage};
use tokio::task::spawn_blocking;

use super::stage;
use super::stage::InstallerAssets;
use crate::error::{Result, WizardError};
use crate::resolve::ResolvedProfile;

/// Builds the compressed initramfs tail (profile + extension EROFS blobs) with fresh extension pulls.
pub async fn build_initramfs_tail(
    resolved_profile: &ResolvedProfile,
    profile_bytes: &[u8],
) -> Result<Vec<u8>> {
    let extensions = pull_extensions(resolved_profile).await?;

    build_tail_from_extensions(&extensions, profile_bytes).await
}

/// Builds the tail and returns the compressed bytes alongside the pulled extensions for caching.
pub async fn build_and_cache_tail(
    resolved_profile: &ResolvedProfile,
    profile_bytes: &[u8],
) -> Result<(Vec<u8>, Vec<(String, PulledImage)>)> {
    let extensions = pull_extensions(resolved_profile).await?;
    let tail = build_tail_from_extensions(&extensions, profile_bytes).await?;

    Ok((tail, extensions))
}

/// Builds the tail from pre-pulled extensions (no network).
async fn build_tail_from_extensions(
    extensions: &[(String, PulledImage)],
    profile_bytes: &[u8],
) -> Result<Vec<u8>> {
    let mut extra_bytes = Vec::new();
    for entry in extensions {
        let blob = erofs_blob_from_image(&entry.1, erofs::DEFAULT_ZSTD_COMPRESSION_LEVEL)?;
        extra_bytes.push((
            format!("extensions/{}.erofs", extension_archive_name(&entry.0)),
            blob,
        ));
    }
    if !profile_bytes.is_empty() {
        extra_bytes.push(("profile.toml".to_owned(), profile_bytes.to_vec()));
    }

    spawn_blocking(move || {
        let mut buf = Vec::new();
        let refs: Vec<(&Path, &[u8])> = extra_bytes
            .iter()
            .map(|entry| (Path::new(entry.0.as_str()), entry.1.as_slice()))
            .collect();
        build_ramune_tail(&refs, &mut buf)?;
        Ok::<_, WizardError>(buf)
    })
    .await
    .map_err(|e| WizardError::BuildError(format!("join initramfs tail task: {e}")))?
}

/// Derives the stable archive base name for an extension.
fn extension_archive_name(name: &str) -> String {
    name.replace('/', "-")
}

async fn pull_extensions(resolved_profile: &ResolvedProfile) -> Result<Vec<(String, PulledImage)>> {
    let resolved_extensions = resolved_profile.extensions();
    if resolved_extensions.is_empty() {
        return Ok(vec![]);
    }

    stage::pull_extensions(resolved_extensions, &resolved_profile.arch(), None)
        .await
        .map_err(|e| WizardError::BuildError(format!("pull extensions: {e}")))
}

fn erofs_blob_from_image(image: &PulledImage, compression_level: i32) -> Result<Vec<u8>> {
    const EROFS_FT_DIR: u8 = 2;
    const EROFS_FT_REG_FILE: u8 = 1;

    let mut entries = vec![TreeEntry {
        rel_path: "/".into(),
        file_type: EROFS_FT_DIR,
        size: 0,
        mode: 0o40755,
        uid: 0,
        gid: 0,
        mtime: 0,
        mtime_nsec: 0,
        symlink_target: vec![],
        rdev: 0,
    }];
    let mut data = HashMap::new();

    for entry in image
        .entries()
        .map_err(|e| WizardError::BuildError(format!("list entries: {e}")))?
    {
        let rel_path = format!("/{}", entry.path().display());
        match entry {
            PulledEntry::File { file, .. } => {
                let mut reader = file
                    .open()
                    .map_err(|e| WizardError::BuildError(format!("open entry: {e}")))?;
                let mut content = Vec::new();
                reader
                    .read_to_end(&mut content)
                    .map_err(|e| WizardError::BuildError(format!("read entry: {e}")))?;
                data.insert(rel_path.clone(), content);
                entries.push(TreeEntry {
                    rel_path,
                    file_type: EROFS_FT_REG_FILE,
                    size: file.len,
                    mode: 0o100_000 | file.mode,
                    uid: 0,
                    gid: 0,
                    mtime: 0,
                    mtime_nsec: 0,
                    symlink_target: vec![],
                    rdev: 0,
                });
            }
            PulledEntry::Dir { mode, .. } => {
                entries.push(TreeEntry {
                    rel_path,
                    file_type: EROFS_FT_DIR,
                    size: 0,
                    mode: 0o040_000 | mode,
                    uid: 0,
                    gid: 0,
                    mtime: 0,
                    mtime_nsec: 0,
                    symlink_target: vec![],
                    rdev: 0,
                });
            }
        }
    }

    let source = erofs::InMemoryTreeSource::new(entries, data);
    let mut buf = Vec::new();
    erofs::mkfs(
        &mut buf,
        &source,
        &erofs::MkfsConfig {
            source_date_epoch: 0,
            file_contexts: None,
            uuid: [0; 16],
            force_uid: Some(0),
            force_gid: Some(0),
            compression: erofs::Compression::Zstd {
                level: compression_level,
            },
        },
    )
    .map_err(|error| WizardError::BuildError(format!("build EROFS blob: {error}")))?;

    Ok(buf)
}

fn build_ramune_tail<W: Write>(entries: &[(&Path, &[u8])], writer: &mut W) -> Result<()> {
    let mut readers: Vec<std::io::Cursor<&[u8]>> = entries
        .iter()
        .map(|&(_, data)| std::io::Cursor::new(data))
        .collect();
    let mut ramune_entries: Vec<ramune::Entry<'_>> = entries
        .iter()
        .zip(readers.iter_mut())
        .map(|(entry, reader)| {
            ramune::Entry::new(
                entry.0,
                0o100_644,
                reader,
                entry.1.len().try_into().unwrap_or(u64::MAX),
            )
        })
        .collect();
    ramune::archive(
        &mut ramune_entries,
        ramune::DEFAULT_ZSTD_COMPRESSION_LEVEL,
        writer,
    )
    .map_err(|e| WizardError::BuildError(format!("build initramfs tail: {e}")))?;

    Ok(())
}

/// Writes the base initramfs followed by a freshly built tail to a `Write` sink.
///
/// Uses cached extensions to avoid re-pulling OCI images.
///
/// # Errors
///
/// Returns an error when reading the base initramfs, building the tail, or writing fails.
pub async fn write_combined_initramfs<W: Write>(
    assets: &InstallerAssets,
    profile_bytes: &[u8],
    cached_extensions: &[(String, PulledImage)],
    writer: &mut W,
) -> Result<()> {
    let mut base_reader = assets
        .initramfs
        .open()
        .map_err(|e| WizardError::BuildError(format!("open initramfs: {e}")))?;
    std::io::copy(&mut base_reader, writer)
        .map_err(|e| WizardError::BuildError(format!("write initramfs base: {e}")))?;

    let tail = build_tail_from_extensions(cached_extensions, profile_bytes).await?;
    writer
        .write_all(&tail)
        .map_err(|e| WizardError::BuildError(format!("write initramfs tail: {e}")))?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use koci::arch::Arch;

    use super::*;
    use crate::request::Platform;

    #[test]
    fn derives_stable_archive_names() {
        // ARRANGE / ACT / ASSERT
        assert_eq!(extension_archive_name("muak-os/qemu"), "muak-os-qemu");
        assert_eq!(
            extension_archive_name("muak-os/iscsi-tools"),
            "muak-os-iscsi-tools"
        );
    }

    #[test]
    fn build_ramune_tail_with_entries() {
        // ARRANGE
        let entries: [(&Path, &[u8]); 2] = [
            (Path::new("profile.toml"), b"profile = true\n"),
            (Path::new("extensions/test.erofs"), b"erofs-bytes"),
        ];

        // ACT
        let mut buf = Vec::new();
        build_ramune_tail(&entries, &mut buf).expect("build tail");

        // ASSERT
        assert!(!buf.is_empty());
    }

    #[tokio::test]
    async fn build_initramfs_tail_without_extensions_appends_profile_tail() {
        // ARRANGE
        let resolved = ResolvedProfile::new(
            Platform::Metal,
            "v1.0.0".into(),
            Arch::Amd64,
            vec![],
            None,
            "ghcr.io/muak-os/installer:v1.0.0".into(),
        );

        // ACT
        let tail = build_initramfs_tail(&resolved, b"profile = true\n")
            .await
            .expect("build initramfs tail");

        // ASSERT
        assert!(!tail.is_empty());
    }

    #[tokio::test]
    async fn pull_extensions_returns_empty_when_no_extensions() {
        // ARRANGE
        let resolved = ResolvedProfile::new(
            Platform::Metal,
            "v1.0.0".into(),
            Arch::Amd64,
            vec![],
            None,
            "ghcr.io/muak-os/installer:v1.0.0".into(),
        );

        // ACT
        let extensions = pull_extensions(&resolved).await.expect("pull extensions");

        // ASSERT
        assert!(extensions.is_empty());
    }
}
