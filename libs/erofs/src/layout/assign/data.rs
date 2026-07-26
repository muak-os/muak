//! Data block address assignment and total image size computation.

use super::super::types::InodeLayout;
use super::{meta_start, sizes};
use crate::checked::{align_up, u32_from_usize};

pub(super) fn data_block_addrs(inodes: &mut [InodeLayout], do_compress: bool) {
    let bs = sizes::block_size();
    let meta_end = compute_meta_end(inodes, do_compress);
    let meta_end_aligned = align_up(meta_end, bs).unwrap_or(meta_end);

    let mut data_offset = meta_end_aligned;
    for inode in inodes {
        if inode.data_blocks > 0 {
            inode.data_blkaddr = data_offset
                .checked_div(bs)
                .and_then(u32_from_usize)
                .unwrap_or_default();
            let block_count = usize::try_from(inode.data_blocks).unwrap_or_default();
            data_offset = data_offset.saturating_add(block_count.saturating_mul(bs));
        }
    }
}

pub(super) fn total_image_size(inodes: &[InodeLayout], do_compress: bool) -> usize {
    let bs = sizes::block_size();
    let mut max_end = meta_start(do_compress);

    for inode in inodes {
        let slot_end =
            sizes::nid_slot_offset(inode.nid).saturating_add(sizes::meta_size_bytes(inode));
        max_end = max_end.max(slot_end);

        if inode.data_blocks > 0 {
            let data_end = usize::try_from(inode.data_blkaddr)
                .unwrap_or_default()
                .saturating_mul(bs)
                .saturating_add(
                    usize::try_from(inode.data_blocks)
                        .unwrap_or_default()
                        .saturating_mul(bs),
                );
            max_end = max_end.max(data_end);
        }
    }

    align_up(max_end, bs).unwrap_or(max_end)
}

fn compute_meta_end(inodes: &[InodeLayout], do_compress: bool) -> usize {
    inodes
        .iter()
        .map(|inode| sizes::nid_slot_offset(inode.nid).saturating_add(sizes::meta_size_bytes(inode)))
        .max()
        .unwrap_or(meta_start(do_compress))
}

#[cfg(test)]
mod tests {
    use std::io;

    use super::{data_block_addrs, total_image_size};
    use crate::BLOCK_SIZE;
    use crate::dir::{EROFS_FT_DIR, EROFS_FT_REG_FILE};
    use crate::inode::EROFS_INODE_FLAT_PLAIN;
    use crate::layout::{InodeLayout, plan};
    use crate::source::SizedFile;
    use crate::testutil::compress_config;
    use crate::tree::TreeEntry;

    #[test]
    fn compressed_data_blkaddr_assigned() {
        // ARRANGE
        let mut zeros_data = vec![0_u8; 8192];
        let mut zeros_cursor = io::Cursor::new(zeros_data.as_mut_slice());
        let files = &mut [
            SizedFile {
                entry: TreeEntry {
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
                reader: &mut io::empty(),
            },
            SizedFile {
                entry: TreeEntry {
                    rel_path: "/zeros".to_owned(),
                    file_type: EROFS_FT_REG_FILE,
                    size: 8192,
                    mode: 0o644,
                    uid: 0,
                    gid: 0,
                    mtime: 0,
                    mtime_nsec: 0,
                    symlink_target: vec![],
                    rdev: 0,
                },
                reader: &mut zeros_cursor,
            },
        ];

        // ACT
        let planned = plan(files, &compress_config(0)).expect("plan");
        let inodes = &planned.inodes;
        let file = inodes
            .iter()
            .find(|inode| inode.rel_path == "/zeros")
            .expect("found");

        // ASSERT
        assert!(file.data_blocks > 0);
        assert!(file.data_blkaddr > 0);
    }

    #[test]
    fn assign_data_and_total_size_handle_large_nid_fallbacks() {
        // ARRANGE
        let inode = InodeLayout {
            rel_path: "/".to_owned(),
            nid: u64::MAX,
            ino: 0,
            mode: 0,
            uid: 0,
            gid: 0,
            mtime: 0,
            mtime_nsec: 0,
            nlink: 1,
            file_type: EROFS_FT_DIR,
            size: 0,
            datalayout: EROFS_INODE_FLAT_PLAIN,
            xattr_payload: Vec::new(),
            xattr_icount: 0,
            raw_data: Vec::new(),
            data_blkaddr: 0,
            data_blocks: 1,
            children: Vec::new(),
            symlink_target: Vec::new(),
            rdev: 0,
            compressed: None,
        };
        let mut inodes = vec![inode];

        // ACT
        data_block_addrs(&mut inodes, false);
        let total_size = total_image_size(&inodes, false);

        // ASSERT
        assert_eq!(inodes.first().expect("root inode").data_blkaddr, 0);
        assert!(total_size >= usize::try_from(BLOCK_SIZE).expect("block size fits usize"));
    }
}
