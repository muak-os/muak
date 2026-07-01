//! Initramfs archive building.

use std::collections::HashMap;
use std::io::{Cursor, Read as _, Write};
use std::path::Path;

use erofs::tree::TreeEntry;
use koci::pulled::{PulledEntry, PulledImage};
use ramune::Entry;
use ramune::EntryStream;

use super::source;
use super::source::InstallerAssets;
use crate::error::{Result, WizardError};
use crate::resolve::BuildPlan;

/// Prebuilt components for the initramfs tail.
#[derive(Clone)]
pub(crate) struct TailParts {
    paths: Vec<String>,
    blobs: Vec<Vec<u8>>,
}

impl TailParts {
    fn is_empty(&self) -> bool {
        self.paths.is_empty()
    }
}

/// Builds EROFS blobs from pre-pulled extensions and profile bytes.
pub(crate) fn prepare_tail_parts(
    extensions: &[(String, PulledImage)],
    profile_bytes: &[u8],
) -> Result<TailParts> {
    let mut paths = Vec::new();
    let mut blobs = Vec::new();
    for entry in extensions {
        let blob = erofs_blob_from_image(&entry.1, erofs::DEFAULT_ZSTD_COMPRESSION_LEVEL)?;
        paths.push(format!(
            "extensions/{}.erofs",
            extension_archive_name(&entry.0)
        ));
        blobs.push(blob);
    }
    if !profile_bytes.is_empty() {
        paths.push("profile.toml".to_owned());
        blobs.push(profile_bytes.to_vec());
    }

    Ok(TailParts { paths, blobs })
}

/// Returns the exact byte length of the raw CPIO archive for a prebuilt tail.
pub(crate) fn tail_exact_size(parts: &TailParts) -> u64 {
    let metas: Vec<Entry> = parts
        .paths
        .iter()
        .zip(parts.blobs.iter())
        .map(|(path, blob)| Entry {
            path: Path::new(path),
            len: u64::try_from(blob.len()).unwrap_or(u64::MAX),
        })
        .collect();

    ramune::raw_size(&metas)
}

/// Writes a raw CPIO archive from prebuilt tail parts into `writer`.
pub(crate) fn build_tail_from_parts(parts: &TailParts, writer: &mut impl Write) -> Result<()> {
    if parts.is_empty() {
        return Ok(());
    }

    let mut readers: Vec<Cursor<&[u8]>> = parts
        .blobs
        .iter()
        .map(|blob| Cursor::new(blob.as_slice()))
        .collect();
    let mut entries: Vec<EntryStream<'_>> = parts
        .paths
        .iter()
        .zip(readers.iter_mut())
        .zip(parts.blobs.iter())
        .map(|((path, reader), blob)| {
            EntryStream::new(
                Path::new(path),
                0o100_644,
                reader,
                u64::try_from(blob.len()).unwrap_or(u64::MAX),
            )
        })
        .collect();

    ramune::raw(&mut entries, writer)
        .map(|_| ())
        .map_err(|e| WizardError::BuildError(format!("build initramfs tail: {e}")))
}

/// Derives the stable archive base name for an extension.
fn extension_archive_name(name: &str) -> String {
    name.replace('/', "-")
}

pub(crate) async fn pull_extensions(
    resolved_profile: &BuildPlan,
) -> Result<Vec<(String, PulledImage)>> {
    let resolved_extensions = resolved_profile.extensions();
    if resolved_extensions.is_empty() {
        return Ok(vec![]);
    }

    source::pull_extensions(resolved_extensions, &resolved_profile.arch(), None)
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
                let mut reader = file.open();
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

/// Writes the base initramfs followed by a freshly built tail to a `Write` sink.
///
/// # Errors
///
/// Returns an error when reading the base initramfs, building the tail, or writing fails.
pub fn write_combined_initramfs<W: Write>(
    assets: &InstallerAssets,
    tail_parts: &TailParts,
    writer: &mut W,
) -> Result<()> {
    let mut base_reader = assets.initramfs.open();
    std::io::copy(&mut base_reader, writer)
        .map_err(|e| WizardError::BuildError(format!("write initramfs base: {e}")))?;

    build_tail_from_parts(tail_parts, writer)
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
    fn build_tail_from_parts_writes_archive() {
        // ARRANGE
        let parts = TailParts {
            paths: vec![
                "profile.toml".to_owned(),
                "extensions/test.erofs".to_owned(),
            ],
            blobs: vec![b"profile = true\n".to_vec(), b"erofs-bytes".to_vec()],
        };

        // ACT
        let mut buf = Vec::new();
        build_tail_from_parts(&parts, &mut buf).expect("build tail");

        // ASSERT
        assert!(!buf.is_empty());
    }

    #[test]
    fn tail_exact_size_matches_built_size() {
        // ARRANGE
        let parts = TailParts {
            paths: vec![
                "profile.toml".to_owned(),
                "extensions/test.erofs".to_owned(),
            ],
            blobs: vec![b"profile = true\n".to_vec(), b"erofs-bytes".to_vec()],
        };

        // ACT
        let expected = tail_exact_size(&parts);
        let mut buf = Vec::new();
        build_tail_from_parts(&parts, &mut buf).expect("build tail");

        // ASSERT
        assert_eq!(expected, u64::try_from(buf.len()).unwrap_or(0));
    }

    #[test]
    fn build_tail_from_parts_empty_returns_ok() {
        // ARRANGE
        let parts = TailParts {
            paths: vec![],
            blobs: vec![],
        };
        let mut buf = Vec::new();

        // ACT
        build_tail_from_parts(&parts, &mut buf).expect("build empty tail");

        // ASSERT
        assert!(buf.is_empty());
    }

    #[tokio::test]
    async fn prepare_tail_parts_builds_nonempty_tail() {
        // ARRANGE
        let resolved = BuildPlan::new(
            Platform::Metal,
            "v1.0.0".into(),
            Arch::Amd64,
            vec![],
            None,
            "ghcr.io/muak-os/installer:v1.0.0".into(),
        );
        // ACT
        let extensions = pull_extensions(&resolved).await.expect("pull extensions");
        let parts =
            prepare_tail_parts(&extensions, b"profile = true\n").expect("prepare tail parts");

        // ASSERT
        assert!(!parts.paths.is_empty());
    }

    #[tokio::test]
    async fn pull_extensions_returns_empty_when_no_extensions() {
        // ARRANGE
        let resolved = BuildPlan::new(
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
