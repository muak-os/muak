//! Initramfs tail archive building.

use std::collections::HashMap;
use std::io::{Cursor, Read as _};
use std::path::Path;

use erofs::tree::TreeEntry;
use koci::pulled::{PulledEntry, PulledImage};
use tokio::task::spawn_blocking;

use super::stage;
use crate::error::{WizardError, Result};
use crate::resolve::ResolvedProfile;

/// Builds the compressed initramfs tail (profile + extension EROFS blobs).
pub async fn build_initramfs_tail(
    resolved_profile: &ResolvedProfile,
    profile_bytes: &[u8],
) -> Result<Vec<u8>> {
    let extensions = pull_extensions(resolved_profile).await?;
    let mut extra_bytes = Vec::new();
    for (name, image) in extensions {
        let blob = erofs_blob_from_image(&image, erofs::DEFAULT_ZSTD_COMPRESSION_LEVEL)?;
        extra_bytes.push((
            format!("extensions/{}.erofs", extension_archive_name(&name)),
            blob,
        ));
    }
    if !profile_bytes.is_empty() {
        extra_bytes.push(("profile.toml".to_owned(), profile_bytes.to_vec()));
    }

    spawn_blocking(move || build_ramune_tail(&extra_bytes))
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

fn build_ramune_tail(entries: &[(String, Vec<u8>)]) -> Result<Vec<u8>> {
    let mut readers: Vec<Cursor<Vec<u8>>> = entries
        .iter()
        .map(|entry| Cursor::new(entry.1.clone()))
        .collect();
    let mut ramune_entries: Vec<ramune::Entry<'_>> = entries
        .iter()
        .zip(readers.iter_mut())
        .map(|(entry, reader)| ramune::Entry {
            archive_path: Path::new(&entry.0),
            mode: 0o100_644,
            len: u64::try_from(entry.1.len()).unwrap_or(u64::MAX),
            reader,
        })
        .collect();
    let mut buf = Vec::new();
    ramune::archive(
        &mut ramune_entries,
        ramune::DEFAULT_ZSTD_COMPRESSION_LEVEL,
        &mut buf,
    )
    .map_err(|e| WizardError::BuildError(format!("build initramfs tail: {e}")))?;

    Ok(buf)
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
        let entries = vec![
            ("profile.toml".to_owned(), b"profile = true\n".to_vec()),
            ("extensions/test.erofs".to_owned(), b"erofs-bytes".to_vec()),
        ];

        // ACT
        let tail = build_ramune_tail(&entries).expect("build tail");

        // ASSERT
        assert!(!tail.is_empty());
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
