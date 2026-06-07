//! Plain and inline data writers for files, directories, and symlinks.

use alloc::collections::BTreeMap;
use std::io::{Seek, Write};

use super::dir::{find_parent_nid, sorted_entries};
use super::util::{block_offset, full_block_bytes, usize_from_u32};
use crate::checked::{add, seek_write, u64_from_usize};
use crate::dir;
use crate::error::{ErofsError, Result};
use crate::inode::EROFS_INODE_FLAT_INLINE;
use crate::layout::InodeLayout;

pub(super) fn inline<W: Write + Seek>(
    writer: &mut W,
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
        seek_write(writer, u64_from_usize(data_start), full_block_data)?;
    }
    let tail_len = data_size.saturating_sub(full_block_bytes);
    if tail_len > 0 {
        let tail_end = add(full_block_bytes, tail_len)
            .ok_or(ErofsError::Internal("inline tail length overflow"))?;
        let tail = data
            .get(full_block_bytes..tail_end)
            .ok_or(ErofsError::Internal("inline tail data out of bounds"))?;
        seek_write(writer, u64_from_usize(inode_header_end), tail)?;
    }

    Ok(())
}

pub(super) fn plain<W: Write + Seek>(
    writer: &mut W,
    data: &[u8],
    data_blkaddr: u32,
    block_size: usize,
) -> Result<()> {
    let data_start = block_offset(data_blkaddr, block_size, "plain data")?;
    seek_write(writer, u64_from_usize(data_start), data)?;
    Ok(())
}

pub(super) fn dir<W: Write + Seek>(
    writer: &mut W,
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
            writer,
            &dir_data,
            inode.data_blocks,
            inode.data_blkaddr,
            usize_from_u32(inode.size),
            inode_header_end,
            block_size,
        )
    } else {
        plain(writer, &dir_data, inode.data_blkaddr, block_size)
    }
}

pub(super) fn symlink<W: Write + Seek>(
    writer: &mut W,
    inode: &InodeLayout,
    inode_header_end: usize,
    block_size: usize,
) -> Result<()> {
    if inode.datalayout == EROFS_INODE_FLAT_INLINE {
        inline(
            writer,
            &inode.symlink_target,
            inode.data_blocks,
            inode.data_blkaddr,
            inode.symlink_target.len(),
            inode_header_end,
            block_size,
        )
    } else {
        plain(
            writer,
            &inode.symlink_target,
            inode.data_blkaddr,
            block_size,
        )
    }
}

pub(super) fn file<W: Write + Seek>(
    writer: &mut W,
    inode: &InodeLayout,
    inode_header_end: usize,
    block_size: usize,
) -> Result<()> {
    let file_data = &inode.raw_data;

    if inode.datalayout == EROFS_INODE_FLAT_INLINE {
        inline(
            writer,
            file_data,
            inode.data_blocks,
            inode.data_blkaddr,
            file_data.len(),
            inode_header_end,
            block_size,
        )?;
    } else {
        plain(writer, file_data, inode.data_blkaddr, block_size)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;

    use crate::inode::{EROFS_INODE_FLAT_INLINE, EROFS_INODE_FLAT_PLAIN};
    use crate::layout;
    use crate::layout::collect::FilesystemTreeSource;
    use crate::testutil::test_config;
    use crate::writer::write_image;

    fn run_write(planned: &layout::ImagePlan, cfg: &crate::MkfsConfig<'_>) -> Vec<u8> {
        let mut cursor = Cursor::new(Vec::new());
        write_image(&mut cursor, planned, cfg).expect("write_image");
        cursor.into_inner()
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
