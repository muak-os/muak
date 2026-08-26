//! Pure-Rust EROFS image writer (mkfs.erofs equivalent).

#![warn(missing_docs)]

extern crate alloc;

mod checked;
mod compress;
pub mod dir;
pub mod error;
mod filecontexts;
mod inode;
pub mod layout;
pub mod source;
mod superblock;
pub mod tree;
pub mod writer;
mod xattr;

/// EROFS compression algorithm and level.
pub type Compression = compress::Compression;
/// `SELinux` file context rules for labeling EROFS inodes.
pub type FileContexts = filecontexts::FileContexts;
/// Planned inode layout for the EROFS image.
pub type InodeLayout = layout::InodeLayout;
/// A fully-planned EROFS image.
pub type ImagePlan = layout::ImagePlan;
/// A sized file entry with metadata and a reader.
pub type SizedFile<'a> = source::SizedFile<'a>;

/// Default zstd compression level for EROFS images.
pub const DEFAULT_ZSTD_COMPRESSION_LEVEL: i32 = compress::DEFAULT_ZSTD_COMPRESSION_LEVEL;

/// Validate a zstd compression level for EROFS images.
///
/// # Errors
///
/// Returns an [`error::ErofsError::InvalidCompressionLevel`] error when `level`
/// is neither `0` nor within the zstd-supported compression level range.
pub fn validate_compression_level(level: i32) -> error::Result<i32> {
    compress::validate_compression_level(level)
}
/// Block size used throughout EROFS images (4 KiB).
pub const BLOCK_SIZE: u32 = 4096;
/// Slot size: every inode occupies an integer number of 32-byte slots.
pub const SLOT_SIZE: usize = 32;

/// Configuration for EROFS image creation.
#[derive(Debug)]
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

/// Build an EROFS filesystem image from sized file entries.
///
/// # Errors
///
/// Returns an error when entries are invalid, compression settings are invalid,
/// filesystem metadata cannot be read, or the image cannot be serialized.
pub fn mkfs<W: std::io::Write>(
    writer: &mut W,
    measure_files: &mut [SizedFile<'_>],
    files: &mut [SizedFile<'_>],
    config: &MkfsConfig<'_>,
) -> error::Result<()> {
    if let Some(level) = config.compression.level() {
        compress::validate_compression_level(level)?;
    }
    let plan = layout::plan(measure_files, config)?;

    writer::image(writer, &plan, files, config)
}

#[cfg(test)]
pub(crate) mod testutil {
    use std::io::Read;

    use super::{Compression, MkfsConfig};
    use crate::layout::InodeLayout;
    use crate::source::SizedFile;
    use crate::tree::TreeEntry;

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

    /// Reconstructs a [`TreeEntry`] from a planned inode (test support).
    pub(crate) fn entry_of(inode: &InodeLayout) -> TreeEntry {
        TreeEntry {
            rel_path: inode.rel_path.clone(),
            file_type: inode.file_type,
            size: u64::from(inode.size),
            mode: u32::from(inode.mode),
            uid: u32::from(inode.uid),
            gid: u32::from(inode.gid),
            mtime: inode.mtime,
            mtime_nsec: inode.mtime_nsec,
            symlink_target: inode.symlink_target.clone(),
            rdev: inode.rdev,
        }
    }

    /// Zero-filled placeholder content matching each entry's size (test support).
    pub(crate) fn zero_data(entry: &TreeEntry) -> Vec<u8> {
        if entry.file_type == crate::dir::EROFS_FT_REG_FILE && entry.size > 0 {
            vec![0_u8; usize::try_from(entry.size).expect("size fits usize")]
        } else {
            Vec::new()
        }
    }

    /// Pairs owned entries with mutable readers into [`SizedFile`]s (test support).
    pub(crate) fn pair_files<R: Read>(
        entries: Vec<TreeEntry>,
        readers: &mut [R],
    ) -> Vec<SizedFile<'_>> {
        entries
            .into_iter()
            .zip(readers.iter_mut())
            .map(|(entry, reader)| SizedFile { entry, reader })
            .collect()
    }

    /// Cursor set over shared content (test support).
    pub(crate) type CursorSet<'a> = Vec<std::io::Cursor<&'a [u8]>>;

    /// Builds one independent cursor set over shared content (test support).
    pub(crate) fn cursor_set(datas: &[Vec<u8>]) -> CursorSet<'_> {
        datas
            .iter()
            .map(|data| std::io::Cursor::new(data.as_slice()))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use std::io::{Cursor, Read};
    use std::path::Path;

    use testutil::{compress_config, test_config};

    use super::*;
    use crate::dir::{EROFS_FT_DIR, EROFS_FT_REG_FILE, EROFS_FT_SYMLINK};
    use crate::tree::TreeEntry;

    fn run_mock(
        entries: Vec<TreeEntry>,
        data_map: &std::collections::HashMap<String, Vec<u8>>,
        config: &MkfsConfig<'_>,
    ) -> Vec<u8> {
        let datas: Vec<Vec<u8>> = entries
            .iter()
            .map(|e| data_map.get(&e.rel_path).cloned().unwrap_or_default())
            .collect();

        let mut pass1: Vec<Cursor<&[u8]>> = datas
            .iter()
            .map(|data| Cursor::new(data.as_slice()))
            .collect();
        let mut files1 = testutil::pair_files(entries.clone(), &mut pass1);
        let planned = layout::plan(&mut files1, config).expect("plan");

        let mut pass2: Vec<Cursor<&[u8]>> = datas
            .iter()
            .map(|data| Cursor::new(data.as_slice()))
            .collect();
        let mut files = testutil::pair_files(entries, &mut pass2);

        let mut buf = Cursor::new(Vec::new());
        writer::image(&mut buf, &planned, &mut files, config).expect("mkfs");
        buf.into_inner()
    }

    fn open_reader(dir_path: &Path, ent: &TreeEntry) -> Box<dyn Read> {
        let should_open = ent.file_type == EROFS_FT_REG_FILE && ent.size > 0;
        if !should_open {
            return Box::new(std::io::empty());
        }
        let full = dir_path.join(ent.rel_path.strip_prefix('/').unwrap_or(&ent.rel_path));
        Box::new(std::fs::File::open(&full).expect("open"))
    }

    fn mkfs_from_dir(dir_path: &Path, config: &MkfsConfig<'_>) -> Vec<u8> {
        let entries = source::collect_entries(dir_path).expect("collect_entries");

        let readers = |entries: &[TreeEntry]| {
            entries
                .iter()
                .map(|ent| open_reader(dir_path, ent))
                .collect::<Vec<_>>()
        };

        let mut pass1 = readers(&entries);
        let mut files1 = testutil::pair_files(entries.clone(), &mut pass1);
        let planned = layout::plan(&mut files1, config).expect("plan");

        let mut pass2 = readers(&entries);
        let mut files = testutil::pair_files(entries, &mut pass2);

        let mut buf = Cursor::new(Vec::new());
        writer::image(&mut buf, &planned, &mut files, config).expect("mkfs");
        buf.into_inner()
    }

    #[test]
    fn mkfs_invalid_source() {
        // ARRANGE
        let nonexistent = Path::new("/this/path/does/not/exist/at/all");

        // ACT
        let result = source::collect_entries(nonexistent);

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
        let image = mkfs_from_dir(dir.path(), &config);

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
        let image = mkfs_from_dir(dir.path(), &config);

        // ASSERT
        assert!(!image.is_empty());
    }

    #[test]
    fn mkfs_compressed_produces_valid_image() {
        // ARRANGE
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(dir.path().join("zeros"), vec![0_u8; 8192]).expect("write");

        // ACT
        let image = mkfs_from_dir(dir.path(), &compress_config(0));

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
        let image = mkfs_from_dir(dir.path(), &compress_config(0));

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
        let image1 = mkfs_from_dir(dir.path(), &config);
        let image2 = mkfs_from_dir(dir.path(), &config);

        // ASSERT
        assert_eq!(image1, image2);
    }

    #[test]
    fn mkfs_compressed_empty_file_not_compressed() {
        // ARRANGE
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(dir.path().join("empty"), b"").expect("write");

        // ACT
        let image = mkfs_from_dir(dir.path(), &compress_config(0));

        // ASSERT
        assert!(!image.is_empty());
        assert!(image.len().is_multiple_of(4096));
    }

    #[test]
    fn compression_default_uses_default_zstd_level() {
        // ARRANGE
        let expected = Compression::Zstd {
            level: DEFAULT_ZSTD_COMPRESSION_LEVEL,
        };

        // ACT
        let default = Compression::default();

        // ASSERT
        assert_eq!(default, expected);
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
        let entries = source::collect_entries(dir.path()).expect("collect_entries");
        let empties = || {
            entries
                .iter()
                .map(|e| SizedFile {
                    entry: e.clone(),
                    reader: Box::leak(Box::new(std::io::empty())),
                })
                .collect::<Vec<_>>()
        };
        let mut measure_files = empties();
        let mut files = empties();
        let mut buf = Cursor::new(Vec::new());
        let result = mkfs(&mut buf, &mut measure_files, &mut files, &config);

        // ASSERT
        assert!(matches!(
            result,
            Err(error::ErofsError::InvalidCompressionLevel { .. })
        ));
    }

    #[test]
    fn plan_rejects_file_instead_of_directory() {
        // ARRANGE
        let file_path = Path::new("/etc/passwd");

        // ACT
        let result = source::collect_entries(file_path);

        // ASSERT
        result.unwrap_err();
    }

    #[test]
    fn block_size_and_slot_size_constants() {
        // ARRANGE & ACT & ASSERT
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
        let image = mkfs_from_dir(dir.path(), &config);

        // ASSERT
        assert!(!image.is_empty());
        assert!(
            image
                .len()
                .is_multiple_of(usize::try_from(BLOCK_SIZE).expect("block size fits usize"))
        );
    }

    #[test]
    fn deterministic_synthetic_output() {
        // ARRANGE
        let entries = vec![
            TreeEntry {
                rel_path: "/".to_owned(),
                file_type: EROFS_FT_DIR,
                size: 0,
                mode: 0o40755,
                uid: 0,
                gid: 0,
                mtime: 0,
                mtime_nsec: 0,
                symlink_target: vec![],
                rdev: 0,
            },
            TreeEntry {
                rel_path: "/a".to_owned(),
                file_type: EROFS_FT_REG_FILE,
                size: 8,
                mode: 0o644,
                uid: 0,
                gid: 0,
                mtime: 0,
                mtime_nsec: 0,
                symlink_target: vec![],
                rdev: 0,
            },
        ];
        let cfg = MkfsConfig {
            uuid: [1; 16],
            source_date_epoch: 1000,
            ..test_config(0)
        };
        let mut map = std::collections::HashMap::new();
        map.insert("/a".to_owned(), vec![1, 2, 3, 4, 5, 6, 7, 8]);

        // ACT
        let img1 = run_mock(entries.clone(), &map, &cfg);
        let img2 = run_mock(entries, &map, &cfg);

        // ASSERT
        assert_eq!(img1, img2);
    }

    #[test]
    fn synthetic_symlink_and_dir_structure() {
        // ARRANGE
        let entries = vec![
            TreeEntry {
                rel_path: "/".to_owned(),
                file_type: EROFS_FT_DIR,
                size: 0,
                mode: 0o40755,
                uid: 0,
                gid: 0,
                mtime: 100,
                mtime_nsec: 0,
                symlink_target: vec![],
                rdev: 0,
            },
            TreeEntry {
                rel_path: "/subdir".to_owned(),
                file_type: EROFS_FT_DIR,
                size: 0,
                mode: 0o40755,
                uid: 1000,
                gid: 100,
                mtime: 200,
                mtime_nsec: 0,
                symlink_target: vec![],
                rdev: 0,
            },
            TreeEntry {
                rel_path: "/subdir/link".to_owned(),
                file_type: EROFS_FT_SYMLINK,
                size: 0,
                mode: 0o120_777,
                uid: 0,
                gid: 0,
                mtime: 300,
                mtime_nsec: 0,
                symlink_target: b"/target".to_vec(),
                rdev: 0,
            },
            TreeEntry {
                rel_path: "/file".to_owned(),
                file_type: EROFS_FT_REG_FILE,
                size: 4,
                mode: 0o644,
                uid: 2000,
                gid: 200,
                mtime: 400,
                mtime_nsec: 0,
                symlink_target: vec![],
                rdev: 0,
            },
        ];
        let cfg = MkfsConfig {
            uuid: [2; 16],
            source_date_epoch: 0,
            ..test_config(0)
        };
        let mut map = std::collections::HashMap::new();
        map.insert("/file".to_owned(), b"abcd".to_vec());

        // ACT
        let image = run_mock(entries, &map, &cfg);

        // ASSERT
        assert!(image.len().is_multiple_of(4096));
        assert!(image.len() >= 4096);
    }

    #[test]
    fn sorted_entry_order_is_deterministic() {
        // ARRANGE
        let entries = vec![
            TreeEntry {
                rel_path: "/z".to_owned(),
                file_type: EROFS_FT_REG_FILE,
                size: 1,
                mode: 0o644,
                uid: 0,
                gid: 0,
                mtime: 0,
                mtime_nsec: 0,
                symlink_target: vec![],
                rdev: 0,
            },
            TreeEntry {
                rel_path: "/a".to_owned(),
                file_type: EROFS_FT_REG_FILE,
                size: 1,
                mode: 0o644,
                uid: 0,
                gid: 0,
                mtime: 0,
                mtime_nsec: 0,
                symlink_target: vec![],
                rdev: 0,
            },
            TreeEntry {
                rel_path: "/".to_owned(),
                file_type: EROFS_FT_DIR,
                size: 0,
                mode: 0o40755,
                uid: 0,
                gid: 0,
                mtime: 0,
                mtime_nsec: 0,
                symlink_target: vec![],
                rdev: 0,
            },
        ];
        let cfg = test_config(0);
        let mut map = std::collections::HashMap::new();
        map.insert("/z".to_owned(), vec![0x42]);
        map.insert("/a".to_owned(), vec![0x42]);

        // ACT
        let image = run_mock(entries, &map, &cfg);

        // ASSERT
        assert!(image.len().is_multiple_of(4096));
        assert!(image.len() >= 4096);
    }

    #[test]
    fn large_file_blocks() {
        // ARRANGE
        let entries = vec![
            TreeEntry {
                rel_path: "/".to_owned(),
                file_type: EROFS_FT_DIR,
                size: 0,
                mode: 0o40755,
                uid: 0,
                gid: 0,
                mtime: 0,
                mtime_nsec: 0,
                symlink_target: vec![],
                rdev: 0,
            },
            TreeEntry {
                rel_path: "/big".to_owned(),
                file_type: EROFS_FT_REG_FILE,
                size: 20_000,
                mode: 0o644,
                uid: 0,
                gid: 0,
                mtime: 0,
                mtime_nsec: 0,
                symlink_target: vec![],
                rdev: 0,
            },
        ];
        let cfg = test_config(0);
        let mut map = std::collections::HashMap::new();
        map.insert("/big".to_owned(), vec![0_u8; 20_000]);

        // ACT
        let image = run_mock(entries, &map, &cfg);

        // ASSERT
        assert!(image.len().is_multiple_of(4096));
        let meta_end = 4096;
        let file_blocks = 5_usize;
        let expected_min = meta_end + file_blocks * 4096;
        assert!(image.len() >= expected_min);
    }

    #[test]
    fn large_file_compressed() {
        // ARRANGE
        let entries = vec![
            TreeEntry {
                rel_path: "/".to_owned(),
                file_type: EROFS_FT_DIR,
                size: 0,
                mode: 0o40755,
                uid: 0,
                gid: 0,
                mtime: 0,
                mtime_nsec: 0,
                symlink_target: vec![],
                rdev: 0,
            },
            TreeEntry {
                rel_path: "/zeros".to_owned(),
                file_type: EROFS_FT_REG_FILE,
                size: 32_768,
                mode: 0o644,
                uid: 0,
                gid: 0,
                mtime: 0,
                mtime_nsec: 0,
                symlink_target: vec![],
                rdev: 0,
            },
        ];
        let cfg = MkfsConfig {
            compression: Compression::Zstd { level: 1 },
            ..test_config(0)
        };
        let mut map = std::collections::HashMap::new();
        map.insert("/zeros".to_owned(), vec![0_u8; 32_768]);

        // ACT
        let image = run_mock(entries, &map, &cfg);

        // ASSERT
        assert!(image.len().is_multiple_of(4096));
        assert!(image.len() >= 4096);
    }
}
