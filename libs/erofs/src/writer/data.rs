//! Plain and inline data serializers for files, directories, and symlinks.

use alloc::borrow::Cow;
use alloc::collections::BTreeMap;

use super::dir::{find_parent_nid, sorted_entries};
use super::util::{block_offset, full_block_bytes, usize_from_u32};
use crate::checked::write_bytes;
use crate::dir;
use crate::dir::{EROFS_FT_DIR, EROFS_FT_REG_FILE, EROFS_FT_SYMLINK};
use crate::error::{ErofsError, Result};
use crate::inode::EROFS_INODE_FLAT_INLINE;
use crate::layout::InodeLayout;

fn plain_data<'a>(
    inode: &'a InodeLayout,
    all_inodes: &'a [InodeLayout],
    path_to_idx: &BTreeMap<String, usize>,
    block_size: usize,
) -> Result<Option<Cow<'a, [u8]>>> {
    match inode.file_type {
        EROFS_FT_DIR => {
            let parent_nid = find_parent_nid(inode, all_inodes, path_to_idx);
            let dir_entries = sorted_entries(inode, all_inodes, path_to_idx, parent_nid);
            let dir_data = dir::serialize_entries(&dir_entries);
            if inode.datalayout == EROFS_INODE_FLAT_INLINE {
                Ok(dir_data
                    .get(..full_block_bytes(inode.data_blocks, block_size)?)
                    .map(|data| Cow::Owned(data.to_vec())))
            } else {
                Ok(Some(Cow::Owned(dir_data)))
            }
        }
        EROFS_FT_SYMLINK => {
            if inode.datalayout == EROFS_INODE_FLAT_INLINE {
                Ok(inode
                    .symlink_target
                    .get(..full_block_bytes(inode.data_blocks, block_size)?)
                    .map(Cow::Borrowed))
            } else {
                Ok(Some(Cow::Borrowed(&inode.symlink_target)))
            }
        }
        EROFS_FT_REG_FILE if inode.compressed.is_none() && inode.size > 0 => {
            if inode.datalayout == EROFS_INODE_FLAT_INLINE {
                Ok(inode
                    .raw_data
                    .get(..full_block_bytes(inode.data_blocks, block_size)?)
                    .map(Cow::Borrowed))
            } else {
                Ok(Some(Cow::Borrowed(&inode.raw_data)))
            }
        }
        _ => Ok(None),
    }
}

pub(super) fn write_inline_tail(
    buf: &mut [u8],
    inode: &InodeLayout,
    all_inodes: &[InodeLayout],
    path_to_idx: &BTreeMap<String, usize>,
    inode_header_end: usize,
    block_size: usize,
) -> Result<()> {
    let full_block_data_len = full_block_bytes(inode.data_blocks, block_size)?;
    match inode.file_type {
        EROFS_FT_DIR => {
            let parent_nid = find_parent_nid(inode, all_inodes, path_to_idx);
            let dir_entries = sorted_entries(inode, all_inodes, path_to_idx, parent_nid);
            let dir_data = dir::serialize_entries(&dir_entries);
            let tail = dir_data
                .get(full_block_data_len..usize_from_u32(inode.size))
                .unwrap_or_default();
            if !tail.is_empty() && !write_bytes(buf, inode_header_end, tail) {
                return Err(ErofsError::Internal("inline tail write out of bounds"));
            }
            Ok(())
        }
        EROFS_FT_SYMLINK => {
            let tail = inode
                .symlink_target
                .get(full_block_data_len..inode.symlink_target.len())
                .unwrap_or_default();
            if !tail.is_empty() && !write_bytes(buf, inode_header_end, tail) {
                return Err(ErofsError::Internal("inline tail write out of bounds"));
            }
            Ok(())
        }
        EROFS_FT_REG_FILE if inode.size > 0 && inode.compressed.is_none() => {
            let tail = inode
                .raw_data
                .get(full_block_data_len..inode.raw_data.len())
                .unwrap_or_default();
            if !tail.is_empty() && !write_bytes(buf, inode_header_end, tail) {
                return Err(ErofsError::Internal("inline tail write out of bounds"));
            }
            Ok(())
        }
        _ => Ok(()),
    }
}

pub(super) fn plain_blocks<'a>(
    inode: &'a InodeLayout,
    all_inodes: &'a [InodeLayout],
    path_to_idx: &BTreeMap<String, usize>,
    block_size: usize,
) -> Result<Option<Cow<'a, [u8]>>> {
    plain_data(inode, all_inodes, path_to_idx, block_size)
}

pub(super) fn write_block_data(
    buf: &mut [u8],
    inode: &InodeLayout,
    data: &[u8],
    block_size: usize,
) -> Result<()> {
    let data_start = block_offset(inode.data_blkaddr, block_size, "plain data")?;
    if !write_bytes(buf, data_start, data) {
        return Err(ErofsError::Internal("plain data write out of bounds"));
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use crate::inode::{EROFS_INODE_FLAT_INLINE, EROFS_INODE_FLAT_PLAIN};
    use crate::layout;
    use crate::layout::collect::FilesystemTreeSource;
    use crate::testutil::test_config;
    use crate::writer::write_image;

    fn run_write(planned: &layout::ImagePlan, cfg: &crate::MkfsConfig<'_>) -> Vec<u8> {
        let mut image = Vec::new();
        write_image(&mut image, planned, cfg).expect("write_image");
        image
    }

    #[test]
    fn write_image_with_inline_file() {
        // ARRANGE
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(dir.path().join("small"), b"hello").expect("write");
        let cfg = test_config(1);

        // ACT
        let planned = layout::plan(&FilesystemTreeSource::new(dir.path()), &cfg).expect("plan");
        let image = run_write(&planned, &cfg);

        // ASSERT
        let file_inode = planned
            .inodes
            .iter()
            .find(|inode| inode.rel_path == "/small")
            .expect("found");
        assert_eq!(file_inode.datalayout, EROFS_INODE_FLAT_INLINE);
        assert!(image.len() >= 4096);
    }

    #[test]
    fn write_dir_data_inline() {
        // ARRANGE
        let dir = tempfile::tempdir().expect("tempdir");
        for index in 0..5_u8 {
            std::fs::write(dir.path().join(format!("f{index}")), [index]).expect("write");
        }
        let cfg = test_config(1);

        // ACT
        let planned = layout::plan(&FilesystemTreeSource::new(dir.path()), &cfg).expect("plan");
        let image = run_write(&planned, &cfg);

        // ASSERT
        let root = planned.inodes.first().expect("root inode");
        assert_eq!(root.datalayout, EROFS_INODE_FLAT_INLINE);
        assert!(image.len() >= 4096);
    }

    #[test]
    fn write_dir_data_plain() {
        // ARRANGE
        let dir = tempfile::tempdir().expect("tempdir");
        for index in 0_u16..339 {
            let name = format!("file_{index:03}.txt");
            std::fs::write(dir.path().join(&name), [index.to_le_bytes()[0]]).expect("write");
        }
        let cfg = test_config(1);

        // ACT
        let planned = layout::plan(&FilesystemTreeSource::new(dir.path()), &cfg).expect("plan");

        // ASSERT
        let root = planned.inodes.first().expect("root inode");
        assert_eq!(root.datalayout, EROFS_INODE_FLAT_PLAIN);
        assert!(root.data_blocks > 0);
    }

    #[test]
    fn write_symlink_data_inline() {
        // ARRANGE
        let dir = tempfile::tempdir().expect("tempdir");
        std::os::unix::fs::symlink("/short", dir.path().join("link")).expect("symlink");
        let cfg = test_config(1);

        // ACT
        let planned = layout::plan(&FilesystemTreeSource::new(dir.path()), &cfg).expect("plan");
        let image = run_write(&planned, &cfg);

        // ASSERT
        let link = planned
            .inodes
            .iter()
            .find(|inode| inode.rel_path == "/link")
            .expect("found");
        assert_eq!(link.datalayout, EROFS_INODE_FLAT_INLINE);
        assert!(image.len() >= 4096);
    }

    #[test]
    fn write_symlink_data_plain() {
        // ARRANGE
        let dir = tempfile::tempdir().expect("tempdir");
        let long_target = "/".to_owned() + &"x".repeat(4080);
        std::os::unix::fs::symlink(&long_target, dir.path().join("longlink")).expect("symlink");
        let cfg = test_config(1);

        // ACT
        let planned = layout::plan(&FilesystemTreeSource::new(dir.path()), &cfg).expect("plan");
        let _: Vec<u8> = run_write(&planned, &cfg);

        // ASSERT
        let link = planned
            .inodes
            .iter()
            .find(|inode| inode.rel_path == "/longlink")
            .expect("found");
        assert_eq!(link.datalayout, EROFS_INODE_FLAT_PLAIN);
        assert!(link.data_blocks > 0);
    }

    #[test]
    fn write_file_data_with_inline_tail() {
        // ARRANGE
        let dir = tempfile::tempdir().expect("tempdir");
        let data = vec![0_u8; 4100];
        std::fs::write(dir.path().join("partial"), &data).expect("write");
        let cfg = test_config(1);

        // ACT
        let planned = layout::plan(&FilesystemTreeSource::new(dir.path()), &cfg).expect("plan");
        let _: Vec<u8> = run_write(&planned, &cfg);

        // ASSERT
        let file = planned
            .inodes
            .iter()
            .find(|inode| inode.rel_path == "/partial")
            .expect("found");
        assert_eq!(file.datalayout, EROFS_INODE_FLAT_INLINE);
        assert!(file.data_blocks > 0);
    }

    #[test]
    fn write_inline_data_only_tail() {
        // ARRANGE
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(dir.path().join("tiny"), b"hi").expect("write");
        let cfg = test_config(1);

        // ACT
        let planned = layout::plan(&FilesystemTreeSource::new(dir.path()), &cfg).expect("plan");
        let _: Vec<u8> = run_write(&planned, &cfg);

        // ASSERT
        let file = planned
            .inodes
            .iter()
            .find(|inode| inode.rel_path == "/tiny")
            .expect("found");
        assert_eq!(file.data_blocks, 0);
    }

    #[test]
    fn write_file_data_plain_layout() {
        // ARRANGE
        let dir = tempfile::tempdir().expect("tempdir");
        let data = vec![0xAB_u8; 4096];
        std::fs::write(dir.path().join("full"), &data).expect("write");
        let cfg = test_config(1);

        // ACT
        let planned = layout::plan(&FilesystemTreeSource::new(dir.path()), &cfg).expect("plan");
        let image = run_write(&planned, &cfg);

        // ASSERT
        let file = planned
            .inodes
            .iter()
            .find(|inode| inode.rel_path == "/full")
            .expect("found");
        assert_eq!(file.datalayout, EROFS_INODE_FLAT_PLAIN);
        let data_start = usize::try_from(file.data_blkaddr).expect("blkaddr fits usize") * 4096;
        assert_eq!(
            image
                .get(data_start..data_start + 4096)
                .expect("plain data bytes"),
            data.as_slice()
        );
    }
}
