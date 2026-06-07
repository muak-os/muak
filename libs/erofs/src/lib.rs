//! Pure-Rust EROFS image writer (mkfs.erofs equivalent).

#![warn(missing_docs)]

extern crate alloc;

mod checked;
mod compress;
mod dir;
mod error;
mod filecontexts;
mod inode;
mod layout;
mod superblock;
pub mod tree;
mod writer;
mod xattr;

/// EROFS compression algorithm and level.
pub type Compression = compress::Compression;
/// Error type for EROFS operations.
pub type ErofsError = error::ErofsError;
/// `SELinux` file context rules for labeling EROFS inodes.
pub type FileContexts = filecontexts::FileContexts;
/// Planned inode layout for the EROFS image.
pub type InodeLayout = layout::InodeLayout;
/// A fully-planned EROFS image.
pub type ImagePlan = layout::ImagePlan;
/// A filesystem-backed [`tree::TreeSource`].
pub type FilesystemTreeSource<'a> = layout::collect::FilesystemTreeSource<'a>;

/// Default zstd compression level for EROFS images.
pub const DEFAULT_ZSTD_COMPRESSION_LEVEL: i32 = compress::DEFAULT_ZSTD_COMPRESSION_LEVEL;
/// Result type alias for EROFS operations.
pub type Result<T> = error::Result<T>;

/// Block size used throughout EROFS images (4 KiB).
pub const BLOCK_SIZE: u32 = 4096;

/// Slot size: every inode occupies an integer number of 32-byte slots.
pub const SLOT_SIZE: usize = 32;

/// Configuration for EROFS image creation.
pub struct MkfsConfig<'a> {
    /// Timestamp for reproducible builds (seconds since epoch).
    pub source_date_epoch: u64,
    /// Optional `SELinux` file context rules.
    pub file_contexts: Option<&'a FileContexts>,
    /// Filesystem UUID.
    pub uuid: [u8; 16],
    /// Override UID for all inodes.
    pub force_uid: Option<u16>,
    /// Override GID for all inodes.
    pub force_gid: Option<u16>,
    /// Compression algorithm and level.
    pub compression: Compression,
}

/// Build an EROFS filesystem image from a source tree.
///
/// # Errors
///
/// Returns an error when the source is invalid, compression settings are invalid,
/// filesystem metadata cannot be read, or the image cannot be serialized.
pub fn mkfs(source: &dyn tree::TreeSource, config: &MkfsConfig<'_>) -> Result<Vec<u8>> {
    if let Some(level) = config.compression.level() {
        compress::validate_compression_level(level)?;
    }
    let plan = layout::plan(source, config)?;
    writer::write_image(&plan, config)
}

#[cfg(test)]
pub(crate) mod testutil {
    use super::{Compression, MkfsConfig};

    /// Returns a minimal uncompressed [`MkfsConfig`] for use in unit tests.
    pub(crate) fn test_config(epoch: u64) -> MkfsConfig<'static> {
        MkfsConfig {
            source_date_epoch: epoch,
            file_contexts: None,
            uuid: [0; 16],
            force_uid: None,
            force_gid: None,
            compression: Compression::None,
        }
    }

    /// Returns a minimal compressed [`MkfsConfig`] for use in unit tests.
    pub(crate) fn compress_config(epoch: u64) -> MkfsConfig<'static> {
        MkfsConfig {
            source_date_epoch: epoch,
            file_contexts: None,
            uuid: [0; 16],
            force_uid: None,
            force_gid: None,
            compression: Compression::default(),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use testutil::{compress_config, test_config};

    use super::*;
    use crate::layout::collect::FilesystemTreeSource;

    #[test]
    fn mkfs_invalid_source() {
        // ARRANGE
        let nonexistent = Path::new("/this/path/does/not/exist/at/all");

        // ACT
        let result = mkfs(&FilesystemTreeSource::new(nonexistent), &test_config(0));

        // ASSERT
        result.unwrap_err();
    }

    #[test]
    fn mkfs_with_selinux() {
        // ARRANGE
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(dir.path().join("f"), b"x").expect("write");
        let fc = FileContexts::from_reader("/.*    system_u:object_r:file_t:s0\n".as_bytes())
            .expect("fc");
        let config = MkfsConfig {
            file_contexts: Some(&fc),
            ..test_config(0)
        };

        // ACT
        let image =
            mkfs(&FilesystemTreeSource::new(dir.path()), &config).expect("mkfs should succeed");

        // ASSERT
        assert!(!image.is_empty());
        assert!(image.len().is_multiple_of(4096));
    }

    #[test]
    fn mkfs_with_force_uid_gid() {
        // ARRANGE
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(dir.path().join("f"), b"x").expect("write");
        let config = MkfsConfig {
            force_uid: Some(1000),
            force_gid: Some(1000),
            ..test_config(0)
        };

        // ACT
        let image =
            mkfs(&FilesystemTreeSource::new(dir.path()), &config).expect("mkfs should succeed");

        // ASSERT
        assert!(!image.is_empty());
    }

    #[test]
    fn mkfs_compressed_produces_valid_image() {
        // ARRANGE
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(dir.path().join("zeros"), vec![0_u8; 8192]).expect("write");

        // ACT
        let image = mkfs(&FilesystemTreeSource::new(dir.path()), &compress_config(0))
            .expect("mkfs should succeed");

        // ASSERT
        assert!(!image.is_empty());
        assert!(image.len().is_multiple_of(4096));
    }

    #[test]
    fn mkfs_compressed_superblock_has_compr_flags() {
        // ARRANGE
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(dir.path().join("zeros"), vec![0_u8; 8192]).expect("write");

        // ACT
        let image =
            mkfs(&FilesystemTreeSource::new(dir.path()), &compress_config(0)).expect("mkfs");

        // ASSERT
        let sb_off = 1024_usize;
        let extslots = *image.get(sb_off + 0x0D).expect("sb_extslots byte");
        assert_eq!(extslots, 0, "sb_extslots must be 0");
        let feature_incompat = u32::from_le_bytes(
            image
                .get(sb_off + 0x50..sb_off + 0x54)
                .expect("feature incompat bytes")
                .try_into()
                .expect("4b"),
        );
        assert_eq!(feature_incompat & 0x02, 0x02, "COMPR_CFGS flag set");
        let avail_algs = u16::from_le_bytes(
            image
                .get(sb_off + 0x54..sb_off + 0x56)
                .expect("available algorithms bytes")
                .try_into()
                .expect("2b"),
        );
        assert_eq!(avail_algs & (1 << 3), 1 << 3, "zstd bit set");
    }

    #[test]
    fn mkfs_compressed_reproducible() {
        // ARRANGE
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(dir.path().join("data"), vec![0_u8; 16384]).expect("write");
        let config = MkfsConfig {
            uuid: [2_u8; 16],
            ..compress_config(1000)
        };

        // ACT
        let image1 = mkfs(&FilesystemTreeSource::new(dir.path()), &config).expect("mkfs 1");
        let image2 = mkfs(&FilesystemTreeSource::new(dir.path()), &config).expect("mkfs 2");

        // ASSERT
        assert_eq!(image1, image2);
    }

    #[test]
    fn mkfs_compressed_empty_file_not_compressed() {
        // ARRANGE
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(dir.path().join("empty"), b"").expect("write");

        // ACT
        let image =
            mkfs(&FilesystemTreeSource::new(dir.path()), &compress_config(0)).expect("mkfs");

        // ASSERT
        assert!(!image.is_empty());
        assert!(image.len().is_multiple_of(4096));
    }

    #[test]
    fn compression_default_uses_default_zstd_level() {
        // ARRANGE
        // ASSERT
        assert_eq!(
            Compression::default(),
            Compression::Zstd {
                level: DEFAULT_ZSTD_COMPRESSION_LEVEL,
            }
        );
    }

    #[test]
    fn mkfs_rejects_invalid_compression_level() {
        // ARRANGE
        let dir = tempfile::tempdir().expect("tempdir");
        let config = MkfsConfig {
            compression: Compression::Zstd { level: i32::MAX },
            ..test_config(0)
        };

        // ACT
        let result = mkfs(&FilesystemTreeSource::new(dir.path()), &config);

        // ASSERT
        assert!(matches!(
            result,
            Err(ErofsError::InvalidCompressionLevel { .. })
        ));
    }

    #[test]
    fn plan_rejects_file_instead_of_directory() {
        // ARRANGE
        let file_path = Path::new("/etc/passwd");

        // ACT
        let result = layout::plan(&FilesystemTreeSource::new(file_path), &test_config(0));

        // ASSERT
        result.unwrap_err();
    }

    #[test]
    fn block_size_and_slot_size_constants() {
        // ARRANGE
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
            uuid: [0xAA_u8; 16],
            ..test_config(0)
        };

        // ACT
        let image = mkfs(&FilesystemTreeSource::new(dir.path()), &config).expect("mkfs");

        // ASSERT
        assert!(!image.is_empty());
        assert!(
            image
                .len()
                .is_multiple_of(usize::try_from(BLOCK_SIZE).expect("block size fits usize"))
        );
    }
}
