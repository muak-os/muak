//! Initial inode construction from filesystem metadata and xattrs.

use std::fs;
use std::os::unix::fs::MetadataExt as _;
use std::path::{Path, PathBuf};

use crate::MkfsConfig;
use crate::dir::{self, EROFS_FT_DIR, EROFS_FT_REG_FILE, EROFS_FT_SYMLINK};
use crate::error::{ErofsError, Result};
use crate::inode::EROFS_INODE_FLAT_PLAIN;
use crate::layout::types::InodeLayout;
use crate::xattr;

/// Build initial `InodeLayout` entries from filesystem metadata.
pub fn initial_inodes(
    entries: &[(PathBuf, String)],
    config: &MkfsConfig<'_>,
) -> Result<Vec<InodeLayout>> {
    let mut inodes = Vec::with_capacity(entries.len());

    for entry in entries {
        let abs = &entry.0;
        let rel = &entry.1;
        let meta = symlink_metadata_with_context(abs)?;

        inode_name(abs, rel)?;

        let file_type = classify_file_type(&meta);
        let epoch = config.source_date_epoch;
        let (mtime, mtime_nsec) = resolve_mtime(&meta, epoch);

        let xattr_payload = config
            .file_contexts
            .and_then(|fc| fc.label_for(rel))
            .map(|label| xattr::selinux_payload(label.as_bytes()))
            .unwrap_or_default();
        let xattr_ic = xattr::icount(xattr_payload.len());

        let symlink_target = read_symlink_target(abs, &meta)?;
        let idx = inodes.len();

        let size = if meta.is_dir() {
            0_u32
        } else {
            truncate_u64_to_u32(meta.len())
        };

        let uid = config
            .force_uid
            .unwrap_or_else(|| truncate_u32_to_u16(meta.uid()));
        let gid = config
            .force_gid
            .unwrap_or_else(|| truncate_u32_to_u16(meta.gid()));

        inodes.push(InodeLayout {
            path: abs.clone(),
            rel_path: rel.clone(),
            nid: 0,
            ino: truncate_usize_to_u32(idx),
            mode: truncate_u32_to_u16(meta.mode()),
            uid,
            gid,
            mtime,
            mtime_nsec,
            nlink: 1,
            file_type,
            size,
            datalayout: EROFS_INODE_FLAT_PLAIN,
            xattr_payload,
            xattr_icount: xattr_ic,
            inline_data: Vec::new(),
            data_blkaddr: 0,
            data_blocks: 0,
            children: Vec::new(),
            symlink_target,
            rdev: if meta.is_dir() || meta.is_file() || meta.is_symlink() {
                0
            } else {
                truncate_u64_to_u32(meta.rdev())
            },
            compressed: None,
        });
    }
    Ok(inodes)
}

/// Classify a filesystem entry's type into an EROFS file type constant.
pub(super) fn classify_file_type(meta: &fs::Metadata) -> u8 {
    if meta.is_dir() {
        EROFS_FT_DIR
    } else if meta.is_symlink() {
        EROFS_FT_SYMLINK
    } else {
        EROFS_FT_REG_FILE
    }
}

pub(super) fn inode_name(abs: &Path, rel: &str) -> Result<String> {
    let name = if rel == "/" {
        String::new()
    } else {
        abs.file_name()
            .map(|name| name.to_string_lossy().to_string())
            .unwrap_or_default()
    };

    if name.len() > dir::EROFS_NAME_LEN {
        return Err(ErofsError::FilenameTooLong(name));
    }

    Ok(name)
}

/// Resolve the modification time, using the source date epoch if configured.
pub(super) fn resolve_mtime(meta: &fs::Metadata, epoch: u64) -> (u64, u32) {
    if epoch > 0 {
        (epoch, 0)
    } else {
        (
            meta.mtime().cast_unsigned(),
            u32::try_from(meta.mtime_nsec()).unwrap_or_default(),
        )
    }
}

/// Read the target of a symbolic link.
pub(super) fn read_symlink_target(abs: &Path, meta: &fs::Metadata) -> Result<Vec<u8>> {
    if !meta.is_symlink() {
        return Ok(Vec::new());
    }
    let target = match fs::read_link(abs) {
        Ok(target) => target,
        Err(_error) => return Err(ErofsError::SymlinkRead(abs.to_path_buf())),
    };
    Ok(target.to_string_lossy().as_bytes().to_vec())
}

pub(super) fn symlink_metadata_with_context(abs: &Path) -> Result<fs::Metadata> {
    fs::symlink_metadata(abs).map_err(|error| {
        ErofsError::Io(std::io::Error::new(
            error.kind(),
            format!("{}: {}", abs.display(), error),
        ))
    })
}

pub(super) fn truncate_u64_to_u32(value: u64) -> u32 {
    let bytes = value.to_le_bytes();
    u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]])
}

pub(super) fn truncate_u32_to_u16(value: u32) -> u16 {
    let bytes = value.to_le_bytes();
    u16::from_le_bytes([bytes[0], bytes[1]])
}

pub(super) fn truncate_usize_to_u32(value: usize) -> u32 {
    let wide = u64::try_from(value).unwrap_or_default();
    truncate_u64_to_u32(wide)
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::{Path, PathBuf};

    use super::{
        classify_file_type, initial_inodes, inode_name, read_symlink_target, resolve_mtime,
        symlink_metadata_with_context, truncate_u32_to_u16, truncate_u64_to_u32,
    };
    use crate::MkfsConfig;
    use crate::dir;
    use crate::error::ErofsError;
    use crate::testutil::test_config;

    #[test]
    fn build_initial_inodes_force_uid_gid() {
        // ARRANGE
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(dir.path().join("f"), b"x").expect("write");
        let entries = super::super::entries(dir.path()).expect("entries");
        let config = MkfsConfig {
            force_uid: Some(1234),
            force_gid: Some(5678),
            ..test_config(0)
        };
        let inodes = initial_inodes(&entries, &config).expect("inodes");
        let file = inodes
            .iter()
            .find(|inode| inode.rel_path == "/f")
            .expect("found");
        // ACT
        // ASSERT
        assert_eq!(file.uid, 1234);
        assert_eq!(file.gid, 5678);
    }

    #[test]
    fn build_initial_inodes_uses_source_date_epoch() {
        // ARRANGE
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(dir.path().join("f"), b"x").expect("write");
        let entries = super::super::entries(dir.path()).expect("entries");
        let inodes = initial_inodes(&entries, &test_config(1_700_000_000)).expect("inodes");
        for inode in &inodes {
            // ACT
            // ASSERT
            assert_eq!(inode.mtime, 1_700_000_000);
            assert_eq!(inode.mtime_nsec, 0);
        }
    }

    #[test]
    fn build_initial_inodes_real_mtime_when_epoch_zero() {
        // ARRANGE
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(dir.path().join("f"), b"x").expect("write");
        let entries = super::super::entries(dir.path()).expect("entries");
        let inodes = initial_inodes(&entries, &test_config(0)).expect("inodes");
        let file = inodes
            .iter()
            .find(|inode| inode.rel_path == "/f")
            .expect("found");
        // ACT
        // ASSERT
        assert!(file.mtime > 0);
    }

    #[test]
    fn build_initial_inodes_symlink_has_target() {
        // ARRANGE
        let dir = tempfile::tempdir().expect("tempdir");
        std::os::unix::fs::symlink("/some/target", dir.path().join("link")).expect("symlink");
        let entries = super::super::entries(dir.path()).expect("entries");
        let inodes = initial_inodes(&entries, &test_config(0)).expect("inodes");
        let link = inodes
            .iter()
            .find(|inode| inode.rel_path == "/link")
            .expect("found");
        // ACT
        // ASSERT
        assert_eq!(link.symlink_target, b"/some/target");
    }

    #[test]
    fn truncate_helpers_drop_high_bits() {
        // ARRANGE
        let wide_u64 = truncate_u64_to_u32(u64::from(u32::MAX).saturating_add(1));
        let wide_u32 = truncate_u32_to_u16(u32::from(u16::MAX).saturating_add(1));
        // ACT
        // ASSERT
        assert_eq!(wide_u64, 0);
        assert_eq!(wide_u32, 0);
    }

    #[test]
    fn symlink_metadata_with_context_reports_path() {
        // ARRANGE
        let missing = Path::new("/definitely/missing/erofs-test-path");
        let result = symlink_metadata_with_context(missing);
        // ACT
        // ASSERT
        assert!(matches!(
            result,
            Err(ErofsError::Io(error)) if error.to_string().contains(missing.to_string_lossy().as_ref())
        ));
    }

    #[test]
    fn inode_name_rejects_overlong_name() {
        // ARRANGE
        let long_name = "a".repeat(dir::EROFS_NAME_LEN + 1);
        let abs = PathBuf::from(format!("/{long_name}"));
        let result = inode_name(&abs, &format!("/{long_name}"));
        // ACT
        // ASSERT
        assert!(matches!(result, Err(ErofsError::FilenameTooLong(name)) if name == long_name));
    }

    #[test]
    fn read_symlink_target_reports_missing_target_path() {
        // ARRANGE
        let dir = tempfile::tempdir().expect("tempdir");
        let link = dir.path().join("link");
        std::os::unix::fs::symlink("target", &link).expect("symlink");
        let meta = fs::symlink_metadata(&link).expect("metadata");
        std::fs::remove_file(&link).expect("remove symlink");
        let result = read_symlink_target(&link, &meta);
        // ACT
        // ASSERT
        assert!(matches!(result, Err(ErofsError::SymlinkRead(path)) if path == link));
    }

    #[test]
    fn build_initial_inodes_sets_rdev_for_special_file() {
        // ARRANGE
        let entries = vec![(PathBuf::from("/dev/null"), "/dev/null".to_owned())];
        let inodes = initial_inodes(&entries, &test_config(0)).expect("inodes");
        // ACT
        // ASSERT
        assert_eq!(inodes.len(), 1);
        assert!(inodes[0].rdev > 0);
    }

    #[test]
    fn resolve_mtime_with_epoch() {
        // ARRANGE
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(dir.path().join("f"), b"x").expect("write");
        let metadata = fs::metadata(dir.path().join("f")).expect("metadata");

        let (mtime, mtime_nsec) = resolve_mtime(&metadata, 1_700_000_000);

        // ACT
        // ASSERT
        assert_eq!(mtime, 1_700_000_000);
        assert_eq!(mtime_nsec, 0);
    }

    #[test]
    fn resolve_mtime_without_epoch() {
        // ARRANGE
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(dir.path().join("f"), b"x").expect("write");
        let metadata = fs::metadata(dir.path().join("f")).expect("metadata");

        let (mtime, _) = resolve_mtime(&metadata, 0);

        // ACT
        // ASSERT
        assert!(mtime > 0);
    }

    #[test]
    fn build_initial_inodes_force_uid() {
        // ARRANGE
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(dir.path().join("f"), b"x").expect("write");
        let entries = super::super::entries(dir.path()).expect("entries");
        let cfg = MkfsConfig {
            force_uid: Some(1000),
            ..test_config(0)
        };

        let inodes = initial_inodes(&entries, &cfg).expect("inodes");
        let file = inodes
            .iter()
            .find(|inode| inode.rel_path == "/f")
            .expect("found");

        // ACT
        // ASSERT
        assert_eq!(file.uid, 1000);
    }

    #[test]
    fn build_initial_inodes_force_gid() {
        // ARRANGE
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(dir.path().join("f"), b"x").expect("write");
        let entries = super::super::entries(dir.path()).expect("entries");
        let cfg = MkfsConfig {
            force_gid: Some(1000),
            ..test_config(0)
        };

        let inodes = initial_inodes(&entries, &cfg).expect("inodes");
        let file = inodes
            .iter()
            .find(|inode| inode.rel_path == "/f")
            .expect("found");

        // ACT
        // ASSERT
        assert_eq!(file.gid, 1000);
    }

    #[test]
    fn classify_file_type_symlink() {
        // ARRANGE
        let dir = tempfile::tempdir().expect("tempdir");
        let link = dir.path().join("link");
        std::os::unix::fs::symlink("/target", &link).expect("symlink");
        let metadata = fs::symlink_metadata(&link).expect("metadata");

        // ACT
        // ASSERT
        assert_eq!(classify_file_type(&metadata), crate::dir::EROFS_FT_SYMLINK);
    }

    #[test]
    fn classify_file_type_regular() {
        // ARRANGE
        let dir = tempfile::tempdir().expect("tempdir");
        let file = dir.path().join("f");
        std::fs::write(&file, b"x").expect("write");
        let metadata = fs::metadata(&file).expect("metadata");

        // ACT
        // ASSERT
        assert_eq!(classify_file_type(&metadata), crate::dir::EROFS_FT_REG_FILE);
    }
}
