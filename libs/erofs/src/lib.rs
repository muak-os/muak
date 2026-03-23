//! Pure-Rust EROFS image writer (mkfs.erofs equivalent).

mod compress;
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

/// Block size used throughout EROFS images (4 KiB).
pub const BLOCK_SIZE: u32 = 4096;
/// Slot size: every inode occupies an integer number of 32-byte slots.
pub const SLOT_SIZE: usize = 32;

/// Configuration for EROFS image creation.
pub struct MkfsConfig<'a> {
    pub source_date_epoch: u64,
    pub file_contexts: Option<&'a FileContexts>,
    pub uuid: [u8; 16],
    pub force_uid: Option<u16>,
    pub force_gid: Option<u16>,
    pub compress: bool,
}

/// Build an EROFS filesystem image from a source directory.
pub fn mkfs(source_dir: &Path, config: &MkfsConfig<'_>) -> Result<Vec<u8>> {
    let inodes = layout::plan(source_dir, config)?;
    writer::write_image(&inodes, config)
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
            compress: false,
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
            compress: false,
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
            compress: false,
        };

        // ACT
        let image = mkfs(dir.path(), &config).expect("mkfs should succeed");

        // ASSERT
        assert!(!image.is_empty());
    }

    #[test]
    fn mkfs_compressed_produces_valid_image() {
        // ARRANGE
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(dir.path().join("zeros"), vec![0u8; 8192]).expect("write");
        let config = MkfsConfig {
            source_date_epoch: 0,
            file_contexts: None,
            uuid: [0; 16],
            force_uid: None,
            force_gid: None,
            compress: true,
        };

        // ACT
        let image = mkfs(dir.path(), &config).expect("mkfs should succeed");

        // ASSERT
        assert!(!image.is_empty());
        assert_eq!(image.len() % 4096, 0);
    }

    #[test]
    fn mkfs_compressed_superblock_has_compr_flags() {
        // ARRANGE
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(dir.path().join("zeros"), vec![0u8; 8192]).expect("write");
        let config = MkfsConfig {
            source_date_epoch: 0,
            file_contexts: None,
            uuid: [0; 16],
            force_uid: None,
            force_gid: None,
            compress: true,
        };

        // ACT
        let image = mkfs(dir.path(), &config).expect("mkfs");

        // ASSERT
        let sb_off = 1024usize;
        let extslots = image[sb_off + 0x0D];
        assert_eq!(extslots, 0, "sb_extslots must be 0");
        let feature_incompat =
            u32::from_le_bytes(image[sb_off + 0x50..sb_off + 0x54].try_into().expect("4b"));
        assert_eq!(feature_incompat & 0x02, 0x02, "COMPR_CFGS flag set");
        let avail_algs =
            u16::from_le_bytes(image[sb_off + 0x54..sb_off + 0x56].try_into().expect("2b"));
        assert_eq!(avail_algs & (1 << 3), 1 << 3, "zstd bit set");
    }

    #[test]
    fn mkfs_compressed_reproducible() {
        // ARRANGE
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(dir.path().join("data"), vec![0u8; 16384]).expect("write");
        let config = MkfsConfig {
            source_date_epoch: 1000,
            file_contexts: None,
            uuid: [2u8; 16],
            force_uid: None,
            force_gid: None,
            compress: true,
        };

        // ACT
        let image1 = mkfs(dir.path(), &config).expect("mkfs 1");
        let image2 = mkfs(dir.path(), &config).expect("mkfs 2");

        // ASSERT
        assert_eq!(image1, image2);
    }

    #[test]
    fn mkfs_compressed_empty_file_not_compressed() {
        // ARRANGE
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(dir.path().join("empty"), b"").expect("write");
        let config = MkfsConfig {
            source_date_epoch: 0,
            file_contexts: None,
            uuid: [0; 16],
            force_uid: None,
            force_gid: None,
            compress: true,
        };

        // ACT
        let image = mkfs(dir.path(), &config).expect("mkfs");

        // ASSERT
        assert!(!image.is_empty());
        assert_eq!(image.len() % 4096, 0);
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
            compress: false,
        };

        // ACT
        let result = layout::plan(file_path, &config);

        // ASSERT
        assert!(result.is_err());
    }

    #[test]
    fn block_size_and_slot_size_constants() {
        // ASSERT
        assert_eq!(BLOCK_SIZE, 4096);
        assert_eq!(SLOT_SIZE, 32);
    }

    #[test]
    fn mkfs_uncompressed_produces_block_aligned_image() {
        // ARRANGE
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(dir.path().join("hello"), b"hello world").expect("write");
        let config = MkfsConfig {
            source_date_epoch: 42,
            file_contexts: None,
            uuid: [0xAAu8; 16],
            force_uid: None,
            force_gid: None,
            compress: false,
        };

        // ACT
        let image = mkfs(dir.path(), &config).expect("mkfs");

        // ASSERT
        assert!(!image.is_empty());
        assert_eq!(image.len() % BLOCK_SIZE as usize, 0);
    }
}
