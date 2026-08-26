//! Inline-tail and streaming plain-data serializers for files, directories, and symlinks.

use alloc::collections::BTreeMap;
use std::io::{Read, Write};

use super::dir::{find_parent_nid, sorted_entries};
use super::sizes::{full_block_bytes, usize_from_u32};
use crate::checked::write_bytes;
use crate::dir;
use crate::dir::{EROFS_FT_DIR, EROFS_FT_SYMLINK};
use crate::error::{ErofsError, Result};
use crate::inode::EROFS_INODE_FLAT_INLINE;
use crate::layout::InodeLayout;

const ZERO_BLOCK: [u8; 4096] = [0_u8; 4096];

/// Write the inline metadata tail of a directory or symlink into the metadata buffer.
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
            write_slice(buf, inode_header_end, &dir_data, full_block_data_len)
        }
        EROFS_FT_SYMLINK => write_slice(
            buf,
            inode_header_end,
            &inode.symlink_target,
            full_block_data_len,
        ),
        _ => Ok(()),
    }
}

/// Write the inline metadata data of an uncompressed regular file.
pub(super) fn write_regular_inline_tail(
    buf: &mut [u8],
    inode: &InodeLayout,
    reader: &mut dyn Read,
    inode_header_end: usize,
) -> Result<()> {
    let size = usize_from_u32(inode.size);
    if size > ZERO_BLOCK.len() {
        return Err(ErofsError::Internal("inline data exceeds block size"));
    }
    let mut tail = ZERO_BLOCK;
    let Some(slot) = tail.get_mut(..size) else {
        return Err(ErofsError::Internal("inline tail slot out of bounds"));
    };
    fill_exact(reader, slot)?;

    write_tail(buf, inode_header_end, slot)
}

/// Stream a regular file's data-phase bytes into the writer and zero-pad to its allocated blocks.
pub(super) fn stream_plain<W: Write>(
    writer: &mut W,
    inode: &InodeLayout,
    reader: &mut dyn Read,
    block_size: usize,
    stage: &mut [u8],
) -> Result<()> {
    let target = full_block_bytes(inode.data_blocks, block_size)?;
    let content = usize_from_u32(inode.size).min(target);
    let mut copied = 0_usize;
    while copied < content {
        let take = content.saturating_sub(copied).min(stage.len());
        let Some(slot) = stage.get_mut(..take) else {
            return Err(ErofsError::Internal("stream stage out of bounds"));
        };
        fill_exact(reader, slot)?;
        writer.write_all(slot).map_err(ErofsError::Io)?;
        copied = copied.saturating_add(take);
    }
    let mut padding = target.saturating_sub(content);
    while padding > 0 {
        let take = padding.min(ZERO_BLOCK.len());
        writer
            .write_all(ZERO_BLOCK.get(..take).unwrap_or(&ZERO_BLOCK))
            .map_err(ErofsError::Io)?;
        padding = padding.saturating_sub(take);
    }

    Ok(())
}

/// Serialized leading blocks of directory or symlink data for the data phase.
pub(super) fn spill_blocks(
    inode: &InodeLayout,
    all_inodes: &[InodeLayout],
    path_to_idx: &BTreeMap<String, usize>,
    block_size: usize,
) -> Result<Option<Vec<u8>>> {
    match inode.file_type {
        EROFS_FT_DIR => {
            let parent_nid = find_parent_nid(inode, all_inodes, path_to_idx);
            let dir_entries = sorted_entries(inode, all_inodes, path_to_idx, parent_nid);
            let dir_data = dir::serialize_entries(&dir_entries);
            if inode.datalayout == EROFS_INODE_FLAT_INLINE {
                Ok(dir_data
                    .get(..full_block_bytes(inode.data_blocks, block_size)?)
                    .map(<[u8]>::to_vec))
            } else {
                Ok(Some(dir_data))
            }
        }
        EROFS_FT_SYMLINK => {
            if inode.datalayout == EROFS_INODE_FLAT_INLINE {
                Ok(inode
                    .symlink_target
                    .get(..full_block_bytes(inode.data_blocks, block_size)?)
                    .map(<[u8]>::to_vec))
            } else {
                Ok(Some(inode.symlink_target.clone()))
            }
        }
        _ => Ok(None),
    }
}

fn write_slice(
    buf: &mut [u8],
    inode_header_end: usize,
    data: &[u8],
    full_block_data_len: usize,
) -> Result<()> {
    let tail = data
        .get(full_block_data_len..data.len())
        .unwrap_or_default();

    write_tail(buf, inode_header_end, tail)
}

fn write_tail(buf: &mut [u8], inode_header_end: usize, tail: &[u8]) -> Result<()> {
    if !tail.is_empty() && !write_bytes(buf, inode_header_end, tail) {
        return Err(ErofsError::Internal("inline tail write out of bounds"));
    }

    Ok(())
}

fn fill_exact(reader: &mut dyn Read, buf: &mut [u8]) -> Result<()> {
    let mut filled = 0_usize;
    while filled < buf.len() {
        let Some(room) = buf.get_mut(filled..) else {
            return Err(ErofsError::Internal("fill window out of bounds"));
        };
        match reader.read(room) {
            Ok(0) => return Err(ErofsError::Internal("file ended before planned size")),
            Ok(count) => filled = filled.saturating_add(count),
            Err(err) if err.kind() == std::io::ErrorKind::Interrupted => {}
            Err(err) => return Err(ErofsError::Io(err)),
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use std::io;

    use crate::dir::{EROFS_FT_DIR, EROFS_FT_REG_FILE, EROFS_FT_SYMLINK};
    use crate::inode::{EROFS_INODE_FLAT_INLINE, EROFS_INODE_FLAT_PLAIN};
    use crate::layout;
    use crate::testutil::{entry_of, test_config, zero_data};
    use crate::tree::TreeEntry;
    use crate::writer;

    fn root_entry() -> TreeEntry {
        TreeEntry {
            rel_path: "/".to_owned(),
            file_type: EROFS_FT_DIR,
            size: 0,
            mode: 0o40755,
            uid: 0,
            gid: 0,
            mtime: 1,
            mtime_nsec: 0,
            symlink_target: vec![],
            rdev: 0,
        }
    }

    fn reg_entry(rel_path: &str, size: u64) -> TreeEntry {
        TreeEntry {
            rel_path: rel_path.to_owned(),
            file_type: EROFS_FT_REG_FILE,
            size,
            mode: 0o644,
            uid: 0,
            gid: 0,
            mtime: 0,
            mtime_nsec: 0,
            symlink_target: vec![],
            rdev: 0,
        }
    }

    fn make_plan(entries: &[TreeEntry], cfg: &crate::MkfsConfig<'_>) -> layout::ImagePlan {
        let mut datas: Vec<Vec<u8>> = entries.iter().map(zero_data).collect();
        let mut cursors: Vec<io::Cursor<&mut [u8]>> = datas
            .iter_mut()
            .map(|data| io::Cursor::new(data.as_mut_slice()))
            .collect();
        let mut files = crate::testutil::pair_files(entries.to_vec(), &mut cursors);
        layout::plan(&mut files, cfg).expect("plan")
    }

    fn write_image(
        planned: &layout::ImagePlan,
        datas: &[Vec<u8>],
        cfg: &crate::MkfsConfig<'_>,
    ) -> Vec<u8> {
        let entries: Vec<TreeEntry> = planned.inodes.iter().map(entry_of).collect();
        let mut cursors = crate::testutil::cursor_set(datas);
        let mut files = crate::testutil::pair_files(entries, &mut cursors);

        let mut buf = Vec::new();
        writer::image(&mut buf, planned, &mut files, cfg).expect("image");
        buf
    }

    #[test]
    fn write_image_with_inline_file() {
        // ARRANGE
        let entries = &[root_entry(), reg_entry("/small", 5)];
        let cfg = test_config(1);

        // ACT
        let planned = make_plan(entries, &cfg);
        let datas: Vec<Vec<u8>> = entries.iter().map(zero_data).collect();
        let image = write_image(&planned, &datas, &cfg);

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
    fn write_dir_data_inline_and_plain() {
        // ARRANGE
        let mut entries = vec![root_entry()];
        for index in 0..5_u8 {
            entries.push(reg_entry(&format!("/f{index}"), 1));
        }
        let cfg = test_config(1);

        // ACT
        let planned = make_plan(&entries, &cfg);

        // ASSERT
        let root = planned.inodes.first().expect("root inode");
        assert_eq!(root.datalayout, EROFS_INODE_FLAT_INLINE);

        let mut many = vec![root_entry()];
        for index in 0_u16..339 {
            many.push(TreeEntry {
                rel_path: format!("/file_{index:03}.txt"),
                file_type: EROFS_FT_REG_FILE,
                size: 1,
                mode: 0o644,
                uid: 0,
                gid: 0,
                mtime: 0,
                mtime_nsec: 0,
                symlink_target: vec![],
                rdev: 0,
            });
        }
        let planned_many = make_plan(&many, &cfg);
        let root_many = planned_many.inodes.first().expect("root inode");
        assert_eq!(root_many.datalayout, EROFS_INODE_FLAT_PLAIN);
        assert!(root_many.data_blocks > 0);
    }

    #[test]
    fn write_symlink_data_inline_and_plain() {
        // ARRANGE
        let short = &[
            root_entry(),
            TreeEntry {
                rel_path: "/link".to_owned(),
                file_type: EROFS_FT_SYMLINK,
                size: 0,
                mode: 0o120_777,
                uid: 0,
                gid: 0,
                mtime: 0,
                mtime_nsec: 0,
                symlink_target: b"/short".to_vec(),
                rdev: 0,
            },
        ];
        let long_target = "/".to_owned() + &"x".repeat(4080);
        let long = &[
            root_entry(),
            TreeEntry {
                rel_path: "/longlink".to_owned(),
                file_type: EROFS_FT_SYMLINK,
                size: 0,
                mode: 0o120_777,
                uid: 0,
                gid: 0,
                mtime: 0,
                mtime_nsec: 0,
                symlink_target: long_target.as_bytes().to_vec(),
                rdev: 0,
            },
        ];
        let cfg = test_config(1);

        // ACT
        let planned_short = make_plan(short, &cfg);
        let planned_long = make_plan(long, &cfg);

        // ASSERT
        let link_short = planned_short.inodes.get(1).expect("short link");
        assert_eq!(link_short.datalayout, EROFS_INODE_FLAT_INLINE);
        let link_long = planned_long.inodes.get(1).expect("long link");
        assert_eq!(link_long.datalayout, EROFS_INODE_FLAT_PLAIN);
        assert!(link_long.data_blocks > 0);
    }

    #[test]
    fn partial_block_file_round_trips_into_data_region() {
        // ARRANGE
        let entries = &[root_entry(), reg_entry("/partial", 4100)];
        let cfg = test_config(1);
        let content: Vec<u8> = (0..251_u8).cycle().take(4100).collect();

        // ACT
        let planned = make_plan(entries, &cfg);
        let datas = vec![Vec::new(), content.clone()];
        let image = write_image(&planned, &datas, &cfg);

        // ASSERT
        let file = planned
            .inodes
            .iter()
            .find(|inode| inode.rel_path == "/partial")
            .expect("found");
        assert_eq!(file.datalayout, EROFS_INODE_FLAT_PLAIN);
        let data_start = usize::try_from(file.data_blkaddr).expect("blkaddr fits usize") * 4096;
        assert_eq!(
            image
                .get(data_start..data_start + 4100)
                .expect("data bytes"),
            content.as_slice()
        );
    }

    #[test]
    fn write_file_data_plain_layout_round_trips() {
        // ARRANGE
        let entries = &[root_entry(), reg_entry("/full", 4096)];
        let cfg = test_config(1);
        let data = vec![0xAB_u8; 4096];

        // ACT
        let planned = make_plan(entries, &cfg);
        let image = write_image(&planned, &[Vec::new(), data.clone()], &cfg);

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
