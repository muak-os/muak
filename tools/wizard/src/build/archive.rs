//! Initramfs archive building.

use std::io::{self, Write};

use erofs::layout;
use erofs::tree::TreeEntry;
use erofs::writer;
use koci::pulled::{PulledEntry, PulledImage};
use ramune::Entry;
use ramune::archive;
use ramune::error::RamuneError;

use super::source;
use super::source::InstallerAssets;
use crate::error::{Result, WizardError};
use crate::resolve::BuildPlan;

pub(crate) struct TailParts {
    entries: Vec<TailEntry>,
}

impl TailParts {
    fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

enum TailEntry {
    Erofs {
        path: String,
        plan: erofs::ImagePlan,
        config: erofs::MkfsConfig<'static>,
        size: u64,
    },
    Raw {
        path: String,
        data: Vec<u8>,
    },
}

pub(crate) fn prepare_tail_parts(
    extensions: &[(String, PulledImage)],
    profile_bytes: &[u8],
) -> Result<TailParts> {
    let mut entries = Vec::new();
    for entry in extensions {
        let (plan, config, size) =
            plan_erofs_from_image(&entry.1, erofs::DEFAULT_ZSTD_COMPRESSION_LEVEL)?;
        entries.push(TailEntry::Erofs {
            path: format!("extensions/{}.erofs", extension_archive_name(&entry.0)),
            plan,
            config,
            size,
        });
    }
    if !profile_bytes.is_empty() {
        entries.push(TailEntry::Raw {
            path: "profile.toml".to_owned(),
            data: profile_bytes.to_vec(),
        });
    }

    Ok(TailParts { entries })
}

pub(crate) fn tail_exact_size(parts: &TailParts) -> u64 {
    let metas: Vec<Entry> = parts
        .entries
        .iter()
        .map(|entry| match *entry {
            TailEntry::Erofs { ref path, size, .. } => Entry {
                path: path.clone(),
                mode: 0o100_644,
                len: size,
            },
            TailEntry::Raw { ref path, ref data } => Entry {
                path: path.clone(),
                mode: 0o100_644,
                len: u64::try_from(data.len()).unwrap_or(u64::MAX),
            },
        })
        .collect();

    archive::size(&metas)
}

pub(crate) fn build_tail_from_parts(parts: &TailParts, writer: &mut impl Write) -> Result<()> {
    if parts.is_empty() {
        return Ok(());
    }

    let mut entries: Vec<Entry> = parts
        .entries
        .iter()
        .map(|entry| match *entry {
            TailEntry::Erofs { ref path, size, .. } => Entry {
                path: path.clone(),
                mode: 0o100_644,
                len: size,
            },
            TailEntry::Raw { ref path, ref data } => Entry {
                path: path.clone(),
                mode: 0o100_644,
                len: u64::try_from(data.len()).unwrap_or(u64::MAX),
            },
        })
        .collect();

    archive::cpio(&mut entries, writer, |entry, w| {
        let tail_entry = parts
            .entries
            .iter()
            .find(|e| entry_path(e) == entry.path)
            .ok_or_else(|| RamuneError::CpioError(format!("unknown tail entry: {}", entry.path)))?;

        write_tail_entry(tail_entry, w).map_err(|e| RamuneError::CpioError(e.to_string()))
    })
    .map(|_| ())
    .map_err(|e| WizardError::BuildError(format!("build initramfs tail: {e}")))
}

fn write_tail_entry<W: Write>(entry: &TailEntry, writer: &mut W) -> Result<()> {
    match *entry {
        TailEntry::Erofs {
            ref plan,
            ref config,
            ..
        } => writer::image(writer, plan, config)
            .map_err(|e| WizardError::BuildError(format!("write EROFS image: {e}"))),
        TailEntry::Raw { ref data, .. } => writer
            .write_all(data)
            .map_err(|e| WizardError::BuildError(format!("write raw data: {e}"))),
    }
}

fn entry_path(entry: &TailEntry) -> &str {
    match *entry {
        TailEntry::Erofs { ref path, .. } | TailEntry::Raw { ref path, .. } => path,
    }
}

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

fn plan_erofs_from_image(
    image: &PulledImage,
    compression_level: i32,
) -> Result<(erofs::ImagePlan, erofs::MkfsConfig<'static>, u64)> {
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
    let mut readers: Vec<Box<dyn io::Read>> = vec![Box::new(io::empty())];

    for entry in image
        .entries()
        .map_err(|e| WizardError::BuildError(format!("list entries: {e}")))?
    {
        let rel_path = format!("/{}", entry.path().display());
        match entry {
            PulledEntry::File { file, .. } => {
                readers.push(Box::new(file.open()));
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
                readers.push(Box::new(io::empty()));
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

    let mut files: Vec<erofs::SizedFile<'_>> = entries
        .into_iter()
        .zip(readers.iter_mut())
        .map(|(entry, reader)| erofs::SizedFile { entry, reader })
        .collect();

    let config = erofs::MkfsConfig {
        source_date_epoch: 0,
        file_contexts: None,
        uuid: [0; 16],
        force_uid: Some(0),
        force_gid: Some(0),
        compression: erofs::Compression::Zstd {
            level: compression_level,
        },
    };

    let plan = layout::plan(&mut files, &config)
        .map_err(|e| WizardError::BuildError(format!("plan EROFS blob: {e}")))?;
    let total_size = u64::try_from(plan.total_size).unwrap_or(u64::MAX);

    Ok((plan, config, total_size))
}

pub fn write_combined_initramfs<W: Write>(
    assets: &InstallerAssets,
    tail_parts: &TailParts,
    writer: &mut W,
) -> Result<()> {
    let mut base_reader = assets.initramfs.open();
    io::copy(&mut base_reader, writer)
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
            entries: vec![
                TailEntry::Raw {
                    path: "profile.toml".to_owned(),
                    data: b"profile = true\n".to_vec(),
                },
                TailEntry::Raw {
                    path: "extensions/test.erofs".to_owned(),
                    data: b"erofs-bytes".to_vec(),
                },
            ],
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
            entries: vec![
                TailEntry::Raw {
                    path: "profile.toml".to_owned(),
                    data: b"profile = true\n".to_vec(),
                },
                TailEntry::Raw {
                    path: "extensions/test.erofs".to_owned(),
                    data: b"erofs-bytes".to_vec(),
                },
            ],
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
        let parts = TailParts { entries: vec![] };
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
        assert!(!parts.entries.is_empty());
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
