//! Pure-Rust EROFS image writer (mkfs.erofs equivalent).

mod dir;
mod error;
mod filecontexts;
mod inode;
mod layout;
mod superblock;
mod writer;
mod xattr;

use std::path::Path;

pub use error::{ErofsError, Result};
pub use filecontexts::FileContexts;
pub use layout::InodeLayout;

pub const BLOCK_SIZE: u32 = 4096;
pub const SLOT_SIZE: usize = 32;

/// Configuration for EROFS image creation.
pub struct MkfsConfig<'a> {
    pub source_date_epoch: u64,
    pub file_contexts: Option<&'a FileContexts>,
    pub uuid: [u8; 16],
    pub force_uid: Option<u16>,
    pub force_gid: Option<u16>,
}

/// Build an EROFS filesystem image from a source directory.
pub fn mkfs(source_dir: &Path, config: &MkfsConfig<'_>) -> Result<Vec<u8>> {
    let inodes = layout::plan(source_dir, config)?;
    writer::write_image(&inodes, config.source_date_epoch, config.uuid)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mkfs_invalid_source() {
        // ARRANGE
        let nonexistent = Path::new("/this/path/does/not/exist/at/all");
        let config = MkfsConfig {
            source_date_epoch: 0,
            file_contexts: None,
            uuid: [0; 16],
            force_uid: None,
            force_gid: None,
        };

        // ACT
        let result = mkfs(nonexistent, &config);

        // ASSERT
        assert!(result.is_err());
    }

    #[test]
    fn mkfs_with_selinux() {
        // ARRANGE
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(dir.path().join("f"), b"x").expect("write");
        let fc = FileContexts::from_reader("/.*    system_u:object_r:file_t:s0\n".as_bytes())
            .expect("fc");
        let config = MkfsConfig {
            source_date_epoch: 0,
            file_contexts: Some(&fc),
            uuid: [0; 16],
            force_uid: None,
            force_gid: None,
        };

        // ACT
        let image = mkfs(dir.path(), &config).expect("mkfs should succeed");

        // ASSERT
        assert!(!image.is_empty());
        assert_eq!(image.len() % 4096, 0);
    }

    #[test]
    fn mkfs_with_force_uid_gid() {
        // ARRANGE
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(dir.path().join("f"), b"x").expect("write");
        let config = MkfsConfig {
            source_date_epoch: 0,
            file_contexts: None,
            uuid: [0; 16],
            force_uid: Some(1000),
            force_gid: Some(1000),
        };

        // ACT
        let image = mkfs(dir.path(), &config).expect("mkfs should succeed");

        // ASSERT
        assert!(!image.is_empty());
    }

    #[test]
    fn plan_rejects_file_instead_of_directory() {
        // ARRANGE
        let file_path = Path::new("/etc/passwd");
        let config = MkfsConfig {
            source_date_epoch: 0,
            file_contexts: None,
            uuid: [0; 16],
            force_uid: None,
            force_gid: None,
        };

        // ACT
        let result = layout::plan(file_path, &config);

        // ASSERT
        assert!(result.is_err());
    }
}
