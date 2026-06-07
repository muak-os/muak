//! Plain and inline data writers for files, directories, and symlinks.

use alloc::collections::BTreeMap;

use super::dir::{find_parent_nid, sorted_entries};
use super::util::{block_offset, full_block_bytes, usize_from_u32};
use crate::checked::{add, write_bytes};
use crate::dir;
use crate::error::{ErofsError, Result};
use crate::inode::EROFS_INODE_FLAT_INLINE;
use crate::layout::InodeLayout;

pub(super) fn inline(
    image: &mut [u8],
    data: &[u8],
    data_blocks: u32,
    data_blkaddr: u32,
    data_size: usize,
    inode_header_end: usize,
    block_size: usize,
) -> Result<()> {
    let full_block_bytes = full_block_bytes(data_blocks, block_size)?;
    if data_blocks > 0 {
        let data_start = block_offset(data_blkaddr, block_size, "inline data")?;
        let full_block_data = data
            .get(..full_block_bytes)
            .ok_or(ErofsError::Internal("inline block data out of bounds"))?;
        if !write_bytes(image, data_start, full_block_data) {
            return Err(ErofsError::Internal("inline block write out of bounds"));
        }
    }
    let tail_len = data_size.saturating_sub(full_block_bytes);
    if tail_len > 0 {
        let tail_end = add(full_block_bytes, tail_len)
            .ok_or(ErofsError::Internal("inline tail length overflow"))?;
        let tail = data
            .get(full_block_bytes..tail_end)
            .ok_or(ErofsError::Internal("inline tail data out of bounds"))?;
        if !write_bytes(image, inode_header_end, tail) {
            return Err(ErofsError::Internal("inline tail write out of bounds"));
        }
    }

    Ok(())
}

pub(super) fn plain(
    image: &mut [u8],
    data: &[u8],
    data_blkaddr: u32,
    block_size: usize,
) -> Result<()> {
    let data_start = block_offset(data_blkaddr, block_size, "plain data")?;
    if write_bytes(image, data_start, data) {
        Ok(())
    } else {
        Err(ErofsError::Internal("plain data write out of bounds"))
    }
}

pub(super) fn dir(
    image: &mut [u8],
    inode: &InodeLayout,
    all_inodes: &[InodeLayout],
    path_to_idx: &BTreeMap<String, usize>,
    inode_header_end: usize,
    block_size: usize,
) -> Result<()> {
    let parent_nid = find_parent_nid(inode, all_inodes, path_to_idx);
    let dir_entries = sorted_entries(inode, all_inodes, path_to_idx, parent_nid);
    let dir_data = dir::serialize_entries(&dir_entries);

    if inode.datalayout == EROFS_INODE_FLAT_INLINE {
        inline(
            image,
            &dir_data,
            inode.data_blocks,
            inode.data_blkaddr,
            usize_from_u32(inode.size),
            inode_header_end,
            block_size,
        )
    } else {
        plain(image, &dir_data, inode.data_blkaddr, block_size)
    }
}

pub(super) fn symlink(
    image: &mut [u8],
    inode: &InodeLayout,
    inode_header_end: usize,
    block_size: usize,
) -> Result<()> {
    if inode.datalayout == EROFS_INODE_FLAT_INLINE {
        inline(
            image,
            &inode.symlink_target,
            inode.data_blocks,
            inode.data_blkaddr,
            inode.symlink_target.len(),
            inode_header_end,
            block_size,
        )
    } else {
        plain(image, &inode.symlink_target, inode.data_blkaddr, block_size)
    }
}

pub(super) fn file(
    image: &mut [u8],
    inode: &InodeLayout,
    inode_header_end: usize,
    block_size: usize,
) -> Result<()> {
    let file_data = &inode.raw_data;

    if inode.datalayout == EROFS_INODE_FLAT_INLINE {
        inline(
            image,
            file_data,
            inode.data_blocks,
            inode.data_blkaddr,
            file_data.len(),
            inode_header_end,
            block_size,
        )?;
    } else {
        plain(image, file_data, inode.data_blkaddr, block_size)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{file, inline, plain};
    use crate::dir::EROFS_FT_REG_FILE;
    use crate::error::ErofsError;
    use crate::inode::{EROFS_INODE_FLAT_INLINE, EROFS_INODE_FLAT_PLAIN};
    use crate::layout::collect::FilesystemTreeSource;
    use crate::layout::{self, InodeLayout};
    use crate::testutil::test_config;
    use crate::writer::write_image;

    #[test]
    fn write_image_with_inline_file() {
        // ARRANGE
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(dir.path().join("small"), b"hello").expect("write");
        let cfg = test_config(1);

        // ACT
        let planned = layout::plan(&FilesystemTreeSource::new(dir.path()), &cfg).expect("plan");
        let image = write_image(&planned, &cfg).expect("write");

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
        let image = write_image(&planned, &cfg).expect("write");

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
        let image = write_image(&planned, &cfg).expect("write");

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
        let _image = write_image(&planned, &cfg).expect("write");

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
        let _image = write_image(&planned, &cfg).expect("write");

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
        let _image = write_image(&planned, &cfg).expect("write");

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
        let image = write_image(&planned, &cfg).expect("write");

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

    #[test]
    fn write_inline_data_reports_out_of_bounds_errors() {
        // ARRANGE
        let mut small_inline_image = [0_u8; 4];

        // ACT
        let block_inline_result = inline(&mut small_inline_image, &[1, 2, 3, 4], 1, 1, 4, 0, 4);
        let tail_inline_result = inline(&mut small_inline_image, &[1, 2, 3], 0, 0, 3, 3, 4);

        // ASSERT
        assert!(matches!(
            block_inline_result,
            Err(ErofsError::Internal("inline block write out of bounds"))
        ));
        assert!(matches!(
            tail_inline_result,
            Err(ErofsError::Internal("inline tail write out of bounds"))
        ));
    }

    #[test]
    fn write_plain_data_reports_out_of_bounds_errors() {
        // ARRANGE
        let mut plain_image = [0_u8; 4];

        // ACT
        let plain_result = plain(&mut plain_image, &[1, 2, 3, 4, 5], 0, 4);

        // ASSERT
        assert!(matches!(
            plain_result,
            Err(ErofsError::Internal("plain data write out of bounds"))
        ));
    }

    #[test]
    fn write_file_data_plain_out_of_bounds() {
        // ARRANGE
        let inode = InodeLayout {
            rel_path: "/data".to_owned(),
            nid: 1,
            ino: 0,
            mode: 0,
            uid: 0,
            gid: 0,
            mtime: 0,
            mtime_nsec: 0,
            nlink: 1,
            file_type: EROFS_FT_REG_FILE,
            size: 0,
            datalayout: EROFS_INODE_FLAT_PLAIN,
            xattr_payload: Vec::new(),
            xattr_icount: 0,
            inline_data: Vec::new(),
            raw_data: vec![0xAB; 8],
            data_blkaddr: 0,
            data_blocks: 0,
            children: Vec::new(),
            symlink_target: Vec::new(),
            rdev: 0,
            compressed: None,
        };
        let mut image = vec![0_u8; 4];

        // ACT
        let result = file(&mut image, &inode, 0, 4096);

        // ASSERT
        assert!(matches!(
            result,
            Err(ErofsError::Internal("plain data write out of bounds"))
        ));
    }
}
