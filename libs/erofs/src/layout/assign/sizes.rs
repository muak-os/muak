//! Inode layout slot sizing and alignment helpers.

use super::super::types::InodeLayout;
use super::compact::index_bytes;
use crate::checked::{align_up, u32_from_usize};
use crate::compress;
use crate::dir::{EROFS_FT_DIR, EROFS_FT_REG_FILE, EROFS_FT_SYMLINK};
use crate::inode::{self, COMPACT_INODE_SIZE, EROFS_INODE_FLAT_INLINE, Z_EROFS_MAP_HEADER_SIZE};
use crate::{BLOCK_SIZE, SLOT_SIZE};

pub(super) fn meta_slots(inode: &InodeLayout) -> usize {
    if let Some(cf) = inode.compressed.as_ref() {
        let totalidx = usize::try_from(compress::lcluster_count(cf)).unwrap_or_default();
        let inode_header = COMPACT_INODE_SIZE.saturating_add(inode.xattr_payload.len());
        let ebase = align8(inode_header).saturating_add(Z_EROFS_MAP_HEADER_SIZE);
        let index_size = index_bytes(totalidx, ebase);
        let total = ebase.saturating_add(index_size);
        total.div_ceil(SLOT_SIZE)
    } else {
        inode::slot_count(
            COMPACT_INODE_SIZE,
            inode.xattr_payload.len(),
            inline_data_size(inode),
        )
    }
}

pub(super) fn align8(val: usize) -> usize {
    align_up(val, 8).unwrap_or(val)
}

pub(super) fn padded_slots(inode_header: usize, inline_len: usize) -> usize {
    let total = inode_header.saturating_add(inline_len);
    align_up(total, SLOT_SIZE).unwrap_or(total)
}

pub(super) fn header_only_padded(inode_header: usize) -> usize {
    align_up(inode_header, SLOT_SIZE).unwrap_or(inode_header)
}

pub(super) fn inline_fits(
    slot_offset: usize,
    inode_header: usize,
    inline_len: usize,
    bs: usize,
) -> bool {
    slot_offset
        .checked_rem(bs)
        .unwrap_or_default()
        .saturating_add(inode_header)
        .saturating_add(inline_len)
        <= bs
}

pub(super) fn inline_data_size(inode: &InodeLayout) -> usize {
    if inode.datalayout != EROFS_INODE_FLAT_INLINE {
        return 0;
    }

    let bs = block_size();
    match inode.file_type {
        EROFS_FT_SYMLINK if inode.data_blocks == 0 => inode.symlink_target.len(),
        EROFS_FT_DIR | EROFS_FT_REG_FILE => {
            let file_size = usize::try_from(inode.size).unwrap_or_default();
            let tail = file_size.checked_rem(bs).unwrap_or_default();
            if tail > 0 && inode.data_blocks == 0 {
                file_size
            } else {
                tail
            }
        }
        _ => 0,
    }
}

pub(super) fn block_size() -> usize {
    usize::try_from(BLOCK_SIZE).unwrap_or_default()
}

pub(super) fn truncate_usize_to_u32(value: usize) -> u32 {
    u32_from_usize(value).unwrap_or_default()
}

pub(super) fn truncate_usize_to_u64(value: usize) -> u64 {
    u64::try_from(value).unwrap_or_default()
}

pub(crate) fn nid_slot_offset(nid: u64) -> usize {
    usize::try_from(nid)
        .unwrap_or_default()
        .saturating_mul(SLOT_SIZE)
}

pub(crate) fn meta_size_bytes(inode: &InodeLayout) -> usize {
    meta_slots(inode).saturating_mul(SLOT_SIZE)
}

#[cfg(test)]
mod tests {
    use super::inline_data_size;
    use crate::dir::{EROFS_FT_REG_FILE, EROFS_FT_SYMLINK};
    use crate::inode::{EROFS_INODE_FLAT_INLINE, EROFS_INODE_FLAT_PLAIN};
    use crate::layout::InodeLayout;

    fn flat_plain_inode(rel_path: &str, file_type: u8) -> InodeLayout {
        InodeLayout {
            rel_path: rel_path.to_owned(),
            nid: 0,
            ino: 0,
            mode: 0,
            uid: 0,
            gid: 0,
            mtime: 0,
            mtime_nsec: 0,
            nlink: 1,
            file_type,
            size: 0,
            datalayout: EROFS_INODE_FLAT_PLAIN,
            xattr_payload: Vec::new(),
            xattr_icount: 0,
            raw_data: Vec::new(),
            data_blkaddr: 0,
            data_blocks: 0,
            children: Vec::new(),
            symlink_target: Vec::new(),
            rdev: 0,
            compressed: None,
        }
    }

    #[test]
    fn inline_data_size_flat_plain_returns_zero() {
        // ARRANGE
        let mut inode = flat_plain_inode("/f", EROFS_FT_REG_FILE);
        inode.size = 100;
        inode.datalayout = EROFS_INODE_FLAT_PLAIN;

        // ACT & ASSERT
        assert_eq!(inline_data_size(&inode), 0);
    }

    #[test]
    fn inline_data_size_symlink_no_blocks() {
        // ARRANGE
        let mut inode = flat_plain_inode("/l", EROFS_FT_SYMLINK);
        inode.datalayout = EROFS_INODE_FLAT_INLINE;
        inode.symlink_target = b"/target".to_vec();
        inode.data_blocks = 0;

        // ACT & ASSERT
        assert_eq!(inline_data_size(&inode), b"/target".len());
    }

    #[test]
    fn inline_data_size_special_file_returns_zero() {
        // ARRANGE
        let mut inode = flat_plain_inode("/dev/null", 0xFF);
        inode.datalayout = EROFS_INODE_FLAT_INLINE;

        // ACT & ASSERT
        assert_eq!(inline_data_size(&inode), 0);
    }

    #[test]
    fn inline_data_size_reg_file_entirely_inline() {
        // ARRANGE
        let mut inode = flat_plain_inode("/f", EROFS_FT_REG_FILE);
        inode.datalayout = EROFS_INODE_FLAT_INLINE;
        inode.size = 100;
        inode.data_blocks = 0;

        // ACT & ASSERT
        assert_eq!(inline_data_size(&inode), 100);
    }

    #[test]
    fn inline_data_size_reg_file_with_tail() {
        // ARRANGE
        let mut inode = flat_plain_inode("/f", EROFS_FT_REG_FILE);
        inode.datalayout = EROFS_INODE_FLAT_INLINE;
        inode.size = 4196;
        inode.data_blocks = 1;

        // ACT & ASSERT
        assert_eq!(inline_data_size(&inode), 100);
    }
}
