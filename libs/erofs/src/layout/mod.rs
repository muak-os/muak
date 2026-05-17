//! Layout planning for inode metadata and data blocks.

mod assign;
mod collect;
mod indices;
mod types;

use std::path::Path;

pub(crate) use assign::compact_index_layout;
pub use assign::total_image_size;
pub use types::InodeLayout;

use crate::MkfsConfig;
use crate::error::{ErofsError, Result};

/// Plan the full image layout from a source directory.
pub fn plan(source_dir: &Path, config: &MkfsConfig<'_>) -> Result<Vec<InodeLayout>> {
    if !source_dir.is_dir() {
        return Err(ErofsError::InvalidSource(source_dir.to_path_buf()));
    }

    let entries = collect::collect_entries(source_dir)?;
    let mut inodes = collect::build_initial_inodes(&entries, config)?;
    let idx = indices::build_indices(&entries, &inodes);

    indices::apply_nlinks(&mut inodes, &idx.nlink_map, &idx.path_to_idx);
    indices::apply_children(&mut inodes, &idx.dir_children, &idx.path_to_idx);
    indices::assign_inos(&mut inodes, &idx.path_to_idx, &idx.dir_children);
    assign::assign_nids_and_layouts(&mut inodes, &idx.path_to_idx, config.compression);
    assign::assign_data_block_addrs(&mut inodes, config.compression.is_enabled());

    Ok(inodes)
}

/// Compute parent relative path from a child relative path.
pub(super) fn parent_rel(rel: &str) -> String {
    if rel == "/" {
        return "/".to_string();
    }
    let s = Path::new(rel)
        .parent()
        .unwrap_or(Path::new("/"))
        .to_string_lossy()
        .to_string();
    if s.is_empty() { "/".to_string() } else { s }
}

#[cfg(test)]
mod tests {
    use std::fs as stdfs;
    use std::os::unix::fs::PermissionsExt;
    use std::path::Path;

    use super::*;
    use crate::MkfsConfig;
    use crate::SLOT_SIZE;
    use crate::dir::{EROFS_FT_DIR, EROFS_FT_REG_FILE, EROFS_FT_SYMLINK};
    use crate::inode::{
        EROFS_INODE_COMPRESSED_COMPACT, EROFS_INODE_FLAT_INLINE, EROFS_INODE_FLAT_PLAIN,
    };
    use crate::testutil::{compress_config, test_config};

    const FIRST_NID: u64 = (assign::META_START / SLOT_SIZE) as u64;

    #[test]
    fn first_nid_is_36() {
        // ASSERT
        assert_eq!(FIRST_NID, 36);
    }

    #[test]
    fn flat_inline_for_small_files() {
        // ARRANGE
        let dir = tempfile::tempdir().expect("tempdir");
        stdfs::write(dir.path().join("small"), b"hello").expect("write");
        stdfs::set_permissions(
            dir.path().join("small"),
            stdfs::Permissions::from_mode(0o644),
        )
        .expect("chmod");

        // ACT
        let inodes = plan(dir.path(), &test_config(1)).expect("plan");
        let file_inode = inodes
            .iter()
            .find(|i| i.rel_path == "/small")
            .expect("found");

        // ASSERT
        assert_eq!(file_inode.datalayout, EROFS_INODE_FLAT_INLINE);
        assert_eq!(file_inode.size, 5);
    }

    #[test]
    fn flat_plain_for_large_files() {
        // ARRANGE
        let dir = tempfile::tempdir().expect("tempdir");
        let data = vec![0u8; 8192];
        stdfs::write(dir.path().join("large"), &data).expect("write");

        // ACT
        let inodes = plan(dir.path(), &test_config(1)).expect("plan");
        let file_inode = inodes
            .iter()
            .find(|i| i.rel_path == "/large")
            .expect("found");

        // ASSERT
        assert_eq!(file_inode.datalayout, EROFS_INODE_FLAT_PLAIN);
        assert_eq!(file_inode.data_blocks, 2);
    }

    #[test]
    fn symlinks_always_inline() {
        // ARRANGE
        let dir = tempfile::tempdir().expect("tempdir");
        std::os::unix::fs::symlink("/target", dir.path().join("link")).expect("symlink");

        // ACT
        let inodes = plan(dir.path(), &test_config(1)).expect("plan");
        let sym = inodes
            .iter()
            .find(|i| i.rel_path == "/link")
            .expect("found");

        // ASSERT
        assert_eq!(sym.datalayout, EROFS_INODE_FLAT_INLINE);
        assert_eq!(sym.file_type, EROFS_FT_SYMLINK);
    }

    #[test]
    fn root_nid_is_36() {
        // ARRANGE
        let dir = tempfile::tempdir().expect("tempdir");

        // ACT
        let inodes = plan(dir.path(), &test_config(1)).expect("plan");

        // ASSERT
        assert_eq!(inodes[0].nid, 36);
    }

    #[test]
    fn nids_assigned_contiguously() {
        // ARRANGE
        let dir = tempfile::tempdir().expect("tempdir");
        stdfs::write(dir.path().join("a"), b"aaa").expect("write");
        stdfs::write(dir.path().join("b"), b"bbb").expect("write");

        // ACT
        let inodes = plan(dir.path(), &test_config(1)).expect("plan");

        // ASSERT
        assert_eq!(inodes[0].nid, 36);
        assert!(inodes[1].nid > inodes[0].nid);
        assert!(inodes[2].nid > inodes[1].nid);
    }

    #[test]
    fn reference_nid_layout() {
        // ARRANGE
        let dir = tempfile::tempdir().expect("tempdir");
        stdfs::write(dir.path().join("hello.txt"), b"world").expect("write");
        std::os::unix::fs::symlink("/target", dir.path().join("link")).expect("symlink");
        stdfs::create_dir(dir.path().join("subdir")).expect("mkdir");
        stdfs::write(dir.path().join("subdir").join("world.txt"), b"hello").expect("write");

        // ACT
        let inodes = plan(dir.path(), &test_config(0)).expect("plan");

        // ASSERT
        assert_eq!(inodes[0].nid, 36, "root NID");
        assert_eq!(inodes[0].file_type, EROFS_FT_DIR);

        let hello = inodes.iter().find(|i| i.rel_path == "/hello.txt");
        assert!(hello.is_some(), "hello.txt found");
        assert_eq!(hello.expect("hello").nid, 40);

        let link = inodes.iter().find(|i| i.rel_path == "/link");
        assert!(link.is_some(), "link found");
        assert_eq!(link.expect("link").nid, 42);

        let subdir = inodes.iter().find(|i| i.rel_path == "/subdir");
        assert!(subdir.is_some(), "subdir found");
        assert_eq!(subdir.expect("subdir").nid, 44);

        let world = inodes.iter().find(|i| i.rel_path == "/subdir/world.txt");
        assert!(world.is_some(), "world.txt found");
        assert_eq!(world.expect("world").nid, 47);
    }

    #[test]
    fn invalid_source_errors() {
        // ARRANGE
        let nonexistent = Path::new("/nonexistent_dir_xyz");

        // ACT
        let result = plan(nonexistent, &test_config(1));

        // ASSERT
        assert!(result.is_err());
    }

    #[test]
    fn layout_dir_inline_single_block() {
        // ARRANGE
        let dir = tempfile::tempdir().expect("tempdir");
        for i in 0..5u8 {
            stdfs::write(dir.path().join(format!("f{i}")), [i]).expect("write");
        }

        // ACT
        let inodes = plan(dir.path(), &test_config(1)).expect("plan");
        let root = &inodes[0];

        // ASSERT
        assert_eq!(root.datalayout, EROFS_INODE_FLAT_INLINE);
        assert_eq!(root.data_blocks, 0);
    }

    #[test]
    fn layout_dir_inline_with_full_blocks() {
        // ARRANGE
        let dir = tempfile::tempdir().expect("tempdir");
        for i in 0..200u8 {
            let name = format!("file_{i:03}.txt");
            stdfs::write(dir.path().join(&name), [i]).expect("write");
        }

        // ACT
        let inodes = plan(dir.path(), &test_config(1)).expect("plan");
        let root = &inodes[0];

        // ASSERT
        assert!(root.data_blocks > 0);
    }

    #[test]
    fn layout_dir_flat_plain() {
        // ARRANGE
        let dir = tempfile::tempdir().expect("tempdir");
        for i in 0u16..339 {
            let name = format!("file_{i:03}.txt");
            stdfs::write(dir.path().join(&name), [i as u8]).expect("write");
        }

        // ACT
        let inodes = plan(dir.path(), &test_config(1)).expect("plan");
        let root = &inodes[0];

        // ASSERT
        assert_eq!(root.datalayout, EROFS_INODE_FLAT_PLAIN);
    }

    #[test]
    fn layout_symlink_inline() {
        // ARRANGE
        let dir = tempfile::tempdir().expect("tempdir");
        std::os::unix::fs::symlink("/short", dir.path().join("link")).expect("symlink");

        // ACT
        let inodes = plan(dir.path(), &test_config(1)).expect("plan");
        let link = inodes
            .iter()
            .find(|i| i.rel_path == "/link")
            .expect("found");

        // ASSERT
        assert_eq!(link.datalayout, EROFS_INODE_FLAT_INLINE);
        assert_eq!(link.data_blocks, 0);
    }

    #[test]
    fn layout_symlink_flat_plain() {
        // ARRANGE
        let dir = tempfile::tempdir().expect("tempdir");
        let long_target = "/".to_string() + &"x".repeat(4080);
        std::os::unix::fs::symlink(&long_target, dir.path().join("longlink")).expect("symlink");

        // ACT
        let inodes = plan(dir.path(), &test_config(1)).expect("plan");
        let link = inodes
            .iter()
            .find(|i| i.rel_path == "/longlink")
            .expect("found");

        // ASSERT
        assert_eq!(link.datalayout, EROFS_INODE_FLAT_PLAIN);
        assert!(link.data_blocks > 0);
    }

    #[test]
    fn layout_regular_empty_file() {
        // ARRANGE
        let dir = tempfile::tempdir().expect("tempdir");
        stdfs::write(dir.path().join("empty"), b"").expect("write");

        // ACT
        let inodes = plan(dir.path(), &test_config(1)).expect("plan");
        let empty = inodes
            .iter()
            .find(|i| i.rel_path == "/empty")
            .expect("found");

        // ASSERT
        assert_eq!(empty.datalayout, EROFS_INODE_FLAT_PLAIN);
        assert_eq!(empty.data_blocks, 0);
        assert_eq!(empty.size, 0);
    }

    #[test]
    fn layout_regular_entirely_inline() {
        // ARRANGE
        let dir = tempfile::tempdir().expect("tempdir");
        stdfs::write(dir.path().join("tiny"), b"hi").expect("write");

        // ACT
        let inodes = plan(dir.path(), &test_config(1)).expect("plan");
        let tiny = inodes
            .iter()
            .find(|i| i.rel_path == "/tiny")
            .expect("found");

        // ASSERT
        assert_eq!(tiny.datalayout, EROFS_INODE_FLAT_INLINE);
        assert_eq!(tiny.data_blocks, 0);
    }

    #[test]
    fn layout_regular_inline_with_full_blocks() {
        // ARRANGE
        let dir = tempfile::tempdir().expect("tempdir");
        let data = vec![0u8; 4100];
        stdfs::write(dir.path().join("partial"), &data).expect("write");

        // ACT
        let inodes = plan(dir.path(), &test_config(1)).expect("plan");
        let partial = inodes
            .iter()
            .find(|i| i.rel_path == "/partial")
            .expect("found");

        // ASSERT
        assert_eq!(partial.datalayout, EROFS_INODE_FLAT_INLINE);
        assert!(partial.data_blocks > 0);
    }

    #[test]
    fn inline_data_size_for_flat_plain() {
        // ARRANGE
        let dir = tempfile::tempdir().expect("tempdir");
        let data = vec![0u8; 8192];
        stdfs::write(dir.path().join("large"), &data).expect("write");

        // ACT
        let inodes = plan(dir.path(), &test_config(1)).expect("plan");
        let large = inodes
            .iter()
            .find(|i| i.rel_path == "/large")
            .expect("found");

        // ASSERT
        assert_eq!(large.datalayout, EROFS_INODE_FLAT_PLAIN);
    }

    #[test]
    fn inline_data_size_for_symlink() {
        // ARRANGE
        let dir = tempfile::tempdir().expect("tempdir");
        std::os::unix::fs::symlink("/target", dir.path().join("link")).expect("symlink");

        // ACT
        let inodes = plan(dir.path(), &test_config(1)).expect("plan");
        let link = inodes
            .iter()
            .find(|i| i.rel_path == "/link")
            .expect("found");

        // ASSERT
        assert_eq!(link.datalayout, EROFS_INODE_FLAT_INLINE);
        assert_eq!(link.data_blocks, 0);
    }

    #[test]
    fn resolve_mtime_with_epoch() {
        // ARRANGE
        let dir = tempfile::tempdir().expect("tempdir");
        stdfs::write(dir.path().join("f"), b"x").expect("write");

        // ACT
        let inodes = plan(dir.path(), &test_config(1700000000)).expect("plan");
        let file = inodes.iter().find(|i| i.rel_path == "/f").expect("found");

        // ASSERT
        assert_eq!(file.mtime, 1700000000);
        assert_eq!(file.mtime_nsec, 0);
    }

    #[test]
    fn resolve_mtime_without_epoch() {
        // ARRANGE
        let dir = tempfile::tempdir().expect("tempdir");
        stdfs::write(dir.path().join("f"), b"x").expect("write");

        // ACT
        let inodes = plan(dir.path(), &test_config(0)).expect("plan");
        let file = inodes.iter().find(|i| i.rel_path == "/f").expect("found");

        // ASSERT
        assert!(file.mtime > 0);
    }

    #[test]
    fn build_initial_inodes_force_uid() {
        // ARRANGE
        let dir = tempfile::tempdir().expect("tempdir");
        stdfs::write(dir.path().join("f"), b"x").expect("write");
        let cfg = MkfsConfig {
            force_uid: Some(1000),
            ..test_config(0)
        };

        // ACT
        let inodes = plan(dir.path(), &cfg).expect("plan");
        let file = inodes.iter().find(|i| i.rel_path == "/f").expect("found");

        // ASSERT
        assert_eq!(file.uid, 1000);
    }

    #[test]
    fn build_initial_inodes_force_gid() {
        // ARRANGE
        let dir = tempfile::tempdir().expect("tempdir");
        stdfs::write(dir.path().join("f"), b"x").expect("write");
        let cfg = MkfsConfig {
            force_gid: Some(1000),
            ..test_config(0)
        };

        // ACT
        let inodes = plan(dir.path(), &cfg).expect("plan");
        let file = inodes.iter().find(|i| i.rel_path == "/f").expect("found");

        // ASSERT
        assert_eq!(file.gid, 1000);
    }

    #[test]
    fn classify_file_type_symlink() {
        // ARRANGE
        let dir = tempfile::tempdir().expect("tempdir");
        std::os::unix::fs::symlink("/target", dir.path().join("link")).expect("symlink");

        // ACT
        let inodes = plan(dir.path(), &test_config(0)).expect("plan");
        let link = inodes
            .iter()
            .find(|i| i.rel_path == "/link")
            .expect("found");

        // ASSERT
        assert_eq!(link.file_type, EROFS_FT_SYMLINK);
    }

    #[test]
    fn classify_file_type_regular() {
        // ARRANGE
        let dir = tempfile::tempdir().expect("tempdir");
        stdfs::write(dir.path().join("f"), b"x").expect("write");

        // ACT
        let inodes = plan(dir.path(), &test_config(0)).expect("plan");
        let file = inodes.iter().find(|i| i.rel_path == "/f").expect("found");

        // ASSERT
        assert_eq!(file.file_type, EROFS_FT_REG_FILE);
    }

    #[test]
    fn readdir_order_nested_directories() {
        // ARRANGE
        let dir = tempfile::tempdir().expect("tempdir");
        stdfs::create_dir(dir.path().join("a")).expect("mkdir");
        stdfs::write(dir.path().join("a").join("b"), b"x").expect("write");

        // ACT
        let inodes = plan(dir.path(), &test_config(0)).expect("plan");

        // ASSERT
        assert!(inodes.iter().any(|i| i.rel_path == "/"));
        assert!(inodes.iter().any(|i| i.rel_path == "/a"));
        assert!(inodes.iter().any(|i| i.rel_path == "/a/b"));
    }

    #[test]
    fn parent_rel_root_is_root() {
        // ACT & ASSERT
        assert_eq!(parent_rel("/"), "/");
    }

    #[test]
    fn parent_rel_nested_path() {
        // ACT & ASSERT
        assert_eq!(parent_rel("/a"), "/");
        assert_eq!(parent_rel("/a/b"), "/a");
        assert_eq!(parent_rel("/a/b/c"), "/a/b");
    }

    #[test]
    fn empty_directory_layout() {
        // ARRANGE
        let dir = tempfile::tempdir().expect("tempdir");

        // ACT
        let inodes = plan(dir.path(), &test_config(1)).expect("plan");
        let root = &inodes[0];

        // ASSERT
        assert_eq!(root.datalayout, EROFS_INODE_FLAT_INLINE);
        assert_eq!(root.data_blocks, 0);
    }

    #[test]
    fn compressed_file_gets_compressed_full_layout() {
        // ARRANGE
        let dir = tempfile::tempdir().expect("tempdir");
        stdfs::write(dir.path().join("zeros"), vec![0u8; 8192]).expect("write");

        // ACT
        let inodes = plan(dir.path(), &compress_config(0)).expect("plan");
        let file = inodes
            .iter()
            .find(|i| i.rel_path == "/zeros")
            .expect("found");

        // ASSERT
        assert_eq!(file.datalayout, EROFS_INODE_COMPRESSED_COMPACT);
        assert!(file.compressed.is_some());
        assert!(file.data_blocks > 0);
    }

    #[test]
    fn compressed_root_nid_shifts_for_extslots() {
        // ARRANGE
        let dir = tempfile::tempdir().expect("tempdir");
        stdfs::write(dir.path().join("zeros"), vec![0u8; 4096]).expect("write");

        // ACT
        let inodes = plan(dir.path(), &compress_config(0)).expect("plan");

        // ASSERT
        let expected_nid = (assign::meta_start(true) / SLOT_SIZE) as u64;
        assert_eq!(inodes[0].nid, expected_nid);
        assert!(inodes[0].nid > FIRST_NID, "nid shifts due to ext slots");
    }

    #[test]
    fn incompressible_file_falls_back_to_flat() {
        // ARRANGE
        let dir = tempfile::tempdir().expect("tempdir");
        let mut state = 0xDEAD_BEEFu32;
        let random_data: Vec<u8> = (0..8192)
            .map(|_| {
                state ^= state << 13;
                state ^= state >> 17;
                state ^= state << 5;
                state as u8
            })
            .collect();
        stdfs::write(dir.path().join("random"), &random_data).expect("write");

        // ACT
        let inodes = plan(dir.path(), &compress_config(0)).expect("plan");
        let file = inodes
            .iter()
            .find(|i| i.rel_path == "/random")
            .expect("found");

        // ASSERT
        assert_ne!(
            file.datalayout, EROFS_INODE_COMPRESSED_COMPACT,
            "incompressible file should not use compressed layout"
        );
        assert!(file.compressed.is_none());
    }

    #[test]
    fn compressed_empty_file_stays_flat() {
        // ARRANGE
        let dir = tempfile::tempdir().expect("tempdir");
        stdfs::write(dir.path().join("empty"), b"").expect("write");

        // ACT
        let inodes = plan(dir.path(), &compress_config(0)).expect("plan");
        let file = inodes
            .iter()
            .find(|i| i.rel_path == "/empty")
            .expect("found");

        // ASSERT
        assert_eq!(file.datalayout, EROFS_INODE_FLAT_PLAIN);
        assert!(file.compressed.is_none());
    }

    #[test]
    fn compressed_small_file_stays_flat_when_no_block_savings() {
        // ARRANGE
        let dir = tempfile::tempdir().expect("tempdir");
        stdfs::write(dir.path().join("small"), vec![0u8; 100]).expect("write");

        // ACT
        let inodes = plan(dir.path(), &compress_config(0)).expect("plan");
        let file = inodes
            .iter()
            .find(|i| i.rel_path == "/small")
            .expect("found");

        // ASSERT
        assert_eq!(file.datalayout, EROFS_INODE_FLAT_INLINE);
        assert!(file.compressed.is_none());
    }

    #[test]
    fn compressed_inode_data_blocks_is_pcluster_count() {
        // ARRANGE
        let dir = tempfile::tempdir().expect("tempdir");
        stdfs::write(dir.path().join("zeros"), vec![0u8; 8192]).expect("write");

        // ACT
        let inodes = plan(dir.path(), &compress_config(0)).expect("plan");
        let file = inodes
            .iter()
            .find(|i| i.rel_path == "/zeros")
            .expect("found");

        // ASSERT
        let cf = file.compressed.as_ref().expect("compressed");
        let pclusters = crate::compress::pcluster_blocks(cf);
        assert_eq!(file.data_blocks, pclusters);
    }

    #[test]
    fn compressed_data_blkaddr_assigned() {
        // ARRANGE
        let dir = tempfile::tempdir().expect("tempdir");
        stdfs::write(dir.path().join("zeros"), vec![0u8; 8192]).expect("write");

        // ACT
        let inodes = plan(dir.path(), &compress_config(0)).expect("plan");
        let file = inodes
            .iter()
            .find(|i| i.rel_path == "/zeros")
            .expect("found");

        // ASSERT
        assert!(file.data_blocks > 0);
        assert!(file.data_blkaddr > 0, "data_blkaddr should be assigned");
    }

    #[test]
    fn mixed_compressed_and_uncompressed_files() {
        // ARRANGE
        let dir = tempfile::tempdir().expect("tempdir");
        stdfs::write(dir.path().join("compressible"), vec![0u8; 8192]).expect("write");
        let mut state = 0xCAFE_BABEu32;
        let random_data: Vec<u8> = (0..8192)
            .map(|_| {
                state ^= state << 13;
                state ^= state >> 17;
                state ^= state << 5;
                state as u8
            })
            .collect();
        stdfs::write(dir.path().join("random"), &random_data).expect("write");

        // ACT
        let inodes = plan(dir.path(), &compress_config(0)).expect("plan");
        let comp = inodes
            .iter()
            .find(|i| i.rel_path == "/compressible")
            .expect("found");
        let rand = inodes
            .iter()
            .find(|i| i.rel_path == "/random")
            .expect("found");

        // ASSERT
        assert_eq!(comp.datalayout, EROFS_INODE_COMPRESSED_COMPACT);
        assert!(comp.compressed.is_some());
        assert_ne!(rand.datalayout, EROFS_INODE_COMPRESSED_COMPACT);
        assert!(rand.compressed.is_none());
    }
}
