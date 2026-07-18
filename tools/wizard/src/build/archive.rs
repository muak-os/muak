//! Initramfs archive building.

use std::io::{self, Write};

use erofs::layout;
use erofs::tree::TreeEntry;
use erofs::writer;
use ramune::Entry;
use ramune::archive;
use ramune::error::RamuneError;

use crate::error::{Result, WizardError};
use crate::source::extension::Metadata as ExtensionMetadata;

pub(crate) enum TailEntry {
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

/// Prepares tail parts from extension metadata and buffered file data.
pub(crate) fn prepare_tail_parts(
    extensions: &[(String, ExtensionMetadata, Vec<Vec<u8>>)],
    profile_bytes: &[u8],
) -> Result<Vec<TailEntry>> {
    let mut entries = Vec::new();
    for ext in extensions {
        let (plan, config, size) =
            plan_erofs_from_metadata(&ext.1.files, &ext.2, erofs::DEFAULT_ZSTD_COMPRESSION_LEVEL)?;
        entries.push(TailEntry::Erofs {
            path: format!("extensions/{}.erofs", extension_archive_name(&ext.0)),
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

    Ok(entries)
}

/// Computes the exact size of the tail archive.
pub(crate) fn tail_exact_size(entries: &[TailEntry]) -> u64 {
    let metas: Vec<Entry> = entries
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

/// Builds the tail archive from prepared parts.
pub(crate) fn build_tail_from_parts(parts: &[TailEntry], writer: &mut impl Write) -> Result<()> {
    if parts.is_empty() {
        return Ok(());
    }

    let mut entries: Vec<Entry> = parts
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

fn plan_erofs_from_metadata(
    files: &[(String, u64, u32)],
    buffered_data: &[Vec<u8>],
    compression_level: i32,
) -> Result<(erofs::ImagePlan, erofs::MkfsConfig<'static>, u64)> {
    const EROFS_FTDIR: u8 = 2;
    const EROFS_FT_REG_FILE: u8 = 1;

    let mut entries = vec![TreeEntry {
        rel_path: "/".into(),
        file_type: EROFS_FTDIR,
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

    for (file_entry, data) in files.iter().zip(buffered_data) {
        let &(ref path, size, mode) = file_entry;
        let rel_path = format!("/{path}");
        readers.push(Box::new(io::Cursor::new(data.clone())));
        entries.push(TreeEntry {
            rel_path,
            file_type: EROFS_FT_REG_FILE,
            size,
            mode: 0o100_000 | mode,
            uid: 0,
            gid: 0,
            mtime: 0,
            mtime_nsec: 0,
            symlink_target: vec![],
            rdev: 0,
        });
    }

    let mut sized_files: Vec<erofs::SizedFile<'_>> = entries
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

    let plan = layout::plan(&mut sized_files, &config)
        .map_err(|e| WizardError::BuildError(format!("plan EROFS blob: {e}")))?;
    let total_size = u64::try_from(plan.total_size).unwrap_or(u64::MAX);

    Ok((plan, config, total_size))
}

#[cfg(test)]
mod tests {
    use super::*;

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
        let parts = vec![
            TailEntry::Raw {
                path: "profile.toml".to_owned(),
                data: b"profile = true\n".to_vec(),
            },
            TailEntry::Raw {
                path: "extensions/test.erofs".to_owned(),
                data: b"erofs-bytes".to_vec(),
            },
        ];

        // ACT
        let mut buf = Vec::new();
        build_tail_from_parts(&parts, &mut buf).expect("build tail");

        // ASSERT
        assert!(!buf.is_empty());
    }

    #[test]
    fn tail_exact_size_matches_built_size() {
        // ARRANGE
        let parts = vec![
            TailEntry::Raw {
                path: "profile.toml".to_owned(),
                data: b"profile = true\n".to_vec(),
            },
            TailEntry::Raw {
                path: "extensions/test.erofs".to_owned(),
                data: b"erofs-bytes".to_vec(),
            },
        ];

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
        let parts: Vec<TailEntry> = vec![];
        let mut buf = Vec::new();

        // ACT
        build_tail_from_parts(&parts, &mut buf).expect("build empty tail");

        // ASSERT
        assert!(buf.is_empty());
    }
}
