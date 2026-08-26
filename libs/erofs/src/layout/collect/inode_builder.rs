//! Initial inode construction from [`TreeEntry`] metadata and xattrs.

use std::path::Path;

use crate::MkfsConfig;
use crate::dir::{self, EROFS_FT_DIR};
use crate::error::{ErofsError, Result};
use crate::inode::EROFS_INODE_FLAT_PLAIN;
use crate::layout::types::InodeLayout;
use crate::tree::TreeEntry;
use crate::xattr;

/// Build initial `InodeLayout` entries from tree entries.
pub fn initial_inodes(entries: &[TreeEntry], config: &MkfsConfig<'_>) -> Result<Vec<InodeLayout>> {
    let mut inodes = Vec::with_capacity(entries.len());

    for (idx, entry) in entries.iter().enumerate() {
        let rel = &entry.rel_path;

        inode_name(rel)?;

        let epoch = config.source_date_epoch;
        let (mtime, mtime_nsec) = if epoch > 0 {
            (epoch, 0)
        } else {
            (entry.mtime, entry.mtime_nsec)
        };

        let xattr_payload = config
            .file_contexts
            .and_then(|fc| fc.label_for(rel))
            .map(|label| xattr::selinux_payload(label.as_bytes()))
            .unwrap_or_default();
        let xattr_ic = xattr::icount(xattr_payload.len());

        let size = if entry.file_type == EROFS_FT_DIR {
            0_u32
        } else {
            truncate_u64_to_u32(entry.size)
        };

        let uid = config
            .force_uid
            .unwrap_or_else(|| truncate_u32_to_u16(entry.uid));
        let gid = config
            .force_gid
            .unwrap_or_else(|| truncate_u32_to_u16(entry.gid));

        inodes.push(InodeLayout {
            rel_path: rel.clone(),
            nid: 0,
            ino: truncate_usize_to_u32(idx),
            mode: truncate_u32_to_u16(entry.mode),
            uid,
            gid,
            mtime,
            mtime_nsec,
            nlink: 1,
            file_type: entry.file_type,
            size,
            datalayout: EROFS_INODE_FLAT_PLAIN,
            xattr_payload,
            xattr_icount: xattr_ic,
            data_blkaddr: 0,
            data_blocks: 0,
            children: Vec::new(),
            symlink_target: entry.symlink_target.clone(),
            rdev: entry.rdev,
            compressed: None,
        });
    }
    Ok(inodes)
}

pub(super) fn inode_name(rel: &str) -> Result<String> {
    let name = if rel == "/" {
        String::new()
    } else {
        Path::new(rel)
            .file_name()
            .map(|name| name.to_string_lossy().to_string())
            .unwrap_or_default()
    };

    if name.len() > dir::EROFS_NAME_LEN {
        return Err(ErofsError::FilenameTooLong(name));
    }

    Ok(name)
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
    use super::{initial_inodes, inode_name, truncate_u32_to_u16, truncate_u64_to_u32};
    use crate::MkfsConfig;
    use crate::dir::{self, EROFS_FT_REG_FILE, EROFS_FT_SYMLINK};
    use crate::error::ErofsError;
    use crate::testutil::test_config;
    use crate::tree::TreeEntry;

    #[test]
    fn build_initial_inodes_force_uid_gid() {
        // ARRANGE
        let entries = vec![TreeEntry {
            rel_path: "/f".to_owned(),
            file_type: EROFS_FT_REG_FILE,
            size: 1,
            mode: 0o644,
            uid: 0,
            gid: 0,
            mtime: 0,
            mtime_nsec: 0,
            symlink_target: vec![],
            rdev: 0,
        }];
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

        // ACT & ASSERT
        assert_eq!(file.uid, 1234);
        assert_eq!(file.gid, 5678);
    }

    #[test]
    fn build_initial_inodes_uses_source_date_epoch() {
        // ARRANGE
        let entries = vec![TreeEntry {
            rel_path: "/f".to_owned(),
            file_type: EROFS_FT_REG_FILE,
            size: 1,
            mode: 0o644,
            uid: 0,
            gid: 0,
            mtime: 0,
            mtime_nsec: 0,
            symlink_target: vec![],
            rdev: 0,
        }];
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
        let entries = vec![TreeEntry {
            rel_path: "/f".to_owned(),
            file_type: EROFS_FT_REG_FILE,
            size: 1,
            mode: 0o644,
            uid: 0,
            gid: 0,
            mtime: 42,
            mtime_nsec: 0,
            symlink_target: vec![],
            rdev: 0,
        }];
        let inodes = initial_inodes(&entries, &test_config(0)).expect("inodes");
        let file = inodes
            .iter()
            .find(|inode| inode.rel_path == "/f")
            .expect("found");

        // ACT & ASSERT
        assert!(file.mtime > 0);
    }

    #[test]
    fn build_initial_inodes_symlink_has_target() {
        // ARRANGE
        let entries = vec![TreeEntry {
            rel_path: "/link".to_owned(),
            file_type: EROFS_FT_SYMLINK,
            size: 0,
            mode: 0o120_777,
            uid: 0,
            gid: 0,
            mtime: 0,
            mtime_nsec: 0,
            symlink_target: b"/some/target".to_vec(),
            rdev: 0,
        }];
        let inodes = initial_inodes(&entries, &test_config(0)).expect("inodes");
        let link = inodes
            .iter()
            .find(|inode| inode.rel_path == "/link")
            .expect("found");

        // ACT & ASSERT
        assert_eq!(link.symlink_target, b"/some/target");
    }

    #[test]
    fn truncate_helpers_drop_high_bits() {
        // ARRANGE
        let wide_u64 = truncate_u64_to_u32(u64::from(u32::MAX).saturating_add(1));
        let wide_u32 = truncate_u32_to_u16(u32::from(u16::MAX).saturating_add(1));

        // ACT & ASSERT
        assert_eq!(wide_u64, 0);
        assert_eq!(wide_u32, 0);
    }

    #[test]
    fn inode_name_rejects_overlong_name() {
        // ARRANGE
        let long_name = "a".repeat(dir::EROFS_NAME_LEN + 1);
        let rel_path = format!("/{long_name}");
        let result = inode_name(&rel_path);

        // ACT & ASSERT
        assert!(matches!(result, Err(ErofsError::FilenameTooLong(name)) if name == long_name));
    }

    #[test]
    fn build_initial_inodes_sets_rdev_for_special_file() {
        // ARRANGE
        let entries = vec![TreeEntry {
            rel_path: "/dev/null".to_owned(),
            file_type: 3,
            size: 0,
            mode: 0o020_666,
            uid: 0,
            gid: 0,
            mtime: 0,
            mtime_nsec: 0,
            symlink_target: Vec::new(),
            rdev: 0x0501,
        }];
        let inodes = initial_inodes(&entries, &test_config(0)).expect("inodes");

        // ACT & ASSERT
        assert_eq!(inodes.len(), 1);
        assert!(inodes.first().expect("device inode").rdev > 0);
    }

    #[test]
    fn build_initial_inodes_force_uid() {
        // ARRANGE
        let entries = vec![TreeEntry {
            rel_path: "/f".to_owned(),
            file_type: EROFS_FT_REG_FILE,
            size: 1,
            mode: 0o644,
            uid: 0,
            gid: 0,
            mtime: 0,
            mtime_nsec: 0,
            symlink_target: vec![],
            rdev: 0,
        }];
        let cfg = MkfsConfig {
            force_uid: Some(1000),
            ..test_config(0)
        };
        let inodes = initial_inodes(&entries, &cfg).expect("inodes");
        let file = inodes
            .iter()
            .find(|inode| inode.rel_path == "/f")
            .expect("found");

        // ACT & ASSERT
        assert_eq!(file.uid, 1000);
    }

    #[test]
    fn build_initial_inodes_force_gid() {
        // ARRANGE
        let entries = vec![TreeEntry {
            rel_path: "/f".to_owned(),
            file_type: EROFS_FT_REG_FILE,
            size: 1,
            mode: 0o644,
            uid: 0,
            gid: 0,
            mtime: 0,
            mtime_nsec: 0,
            symlink_target: vec![],
            rdev: 0,
        }];
        let cfg = MkfsConfig {
            force_gid: Some(1000),
            ..test_config(0)
        };
        let inodes = initial_inodes(&entries, &cfg).expect("inodes");
        let file = inodes
            .iter()
            .find(|inode| inode.rel_path == "/f")
            .expect("found");

        // ACT & ASSERT
        assert_eq!(file.gid, 1000);
    }
}
