//! NID and data layout assignment for each inode in BFS order.

use std::collections::{BTreeMap, VecDeque};
use std::path::Path;

use super::parent_rel;
use super::types::InodeLayout;
use crate::compress;
use crate::dir::{self, DirEntry, EROFS_FT_DIR, EROFS_FT_REG_FILE, EROFS_FT_SYMLINK};
use crate::inode::{
    self, COMPACT_INODE_SIZE, EROFS_INODE_COMPRESSED_COMPACT, EROFS_INODE_FLAT_INLINE,
    EROFS_INODE_FLAT_PLAIN, Z_EROFS_MAP_HEADER_SIZE,
};
use crate::superblock::EROFS_SUPER_OFFSET;
use crate::{BLOCK_SIZE, SLOT_SIZE};

/// Byte offset at which the inode metadata region begins (no compression).
pub(super) const META_START: usize = EROFS_SUPER_OFFSET + 128;

/// Number of 16-byte ext slots reserved for compression config.
pub(super) const COMPR_CFG_EXTSLOTS: usize = 1;

/// Byte offset at which the inode metadata region begins.
pub(super) fn meta_start(has_compression: bool) -> usize {
    let base = if has_compression {
        META_START + COMPR_CFG_EXTSLOTS * 16
    } else {
        META_START
    };
    base.div_ceil(SLOT_SIZE) * SLOT_SIZE
}

/// Assign NIDs and decide data layout for each inode.
pub fn assign_nids_and_layouts(
    inodes: &mut [InodeLayout],
    path_to_idx: &BTreeMap<String, usize>,
    do_compress: bool,
) {
    let bs = BLOCK_SIZE as usize;
    let mut meta_offset = meta_start(do_compress);
    let visit_order = bfs_order(inodes, path_to_idx);

    for i in visit_order {
        let slot_offset = meta_offset;
        let nid = (meta_offset / SLOT_SIZE) as u64;
        let xattr_size = inodes[i].xattr_payload.len();
        let inode_header = COMPACT_INODE_SIZE + xattr_size;

        let advance = match inodes[i].file_type {
            EROFS_FT_DIR => layout_dir(inodes, i, nid, slot_offset, inode_header, path_to_idx, bs),
            EROFS_FT_SYMLINK => layout_symlink(inodes, i, nid, slot_offset, inode_header, bs),
            EROFS_FT_REG_FILE => {
                layout_regular(inodes, i, nid, slot_offset, inode_header, bs, do_compress)
            }
            _ => layout_special(inodes, i, nid, inode_header),
        };
        meta_offset += advance;
    }
}

/// Assign data block addresses after all NIDs are computed.
pub fn assign_data_block_addrs(inodes: &mut [InodeLayout], do_compress: bool) {
    let bs = BLOCK_SIZE as usize;
    let meta_end = compute_meta_end(inodes, do_compress);
    let meta_end_aligned = meta_end.div_ceil(bs) * bs;

    let mut data_offset = meta_end_aligned;
    for inode in inodes {
        if inode.data_blocks > 0 {
            inode.data_blkaddr = (data_offset / bs) as u32;
            data_offset += inode.data_blocks as usize * bs;
        }
    }
}

/// Compute the total image size from the layout.
pub fn total_image_size(inodes: &[InodeLayout], do_compress: bool) -> usize {
    let bs = BLOCK_SIZE as usize;
    let mut max_end = meta_start(do_compress);

    for inode in inodes {
        let slot_end = inode.nid as usize * SLOT_SIZE + meta_slots(inode) * SLOT_SIZE;
        max_end = max_end.max(slot_end);

        if inode.data_blocks > 0 {
            let data_end = inode.data_blkaddr as usize * bs + inode.data_blocks as usize * bs;
            max_end = max_end.max(data_end);
        }
    }

    max_end.div_ceil(bs) * bs
}

/// Compute the byte offset just past the last inode's meta region.
fn compute_meta_end(inodes: &[InodeLayout], do_compress: bool) -> usize {
    inodes
        .iter()
        .map(|inode| inode.nid as usize * SLOT_SIZE + meta_slots(inode) * SLOT_SIZE)
        .max()
        .unwrap_or(meta_start(do_compress))
}

/// Number of 32-byte slots an inode's metadata occupies.
fn meta_slots(inode: &InodeLayout) -> usize {
    if let Some(cf) = &inode.compressed {
        let totalidx = compress::lcluster_count(cf) as usize;
        let inode_header = COMPACT_INODE_SIZE + inode.xattr_payload.len();
        let ebase = align8(inode_header) + Z_EROFS_MAP_HEADER_SIZE;
        let index_size = compact_index_bytes(totalidx, ebase);
        let total = ebase + index_size;
        total.div_ceil(SLOT_SIZE)
    } else {
        inode::slot_count(
            COMPACT_INODE_SIZE,
            inode.xattr_payload.len(),
            inline_data_size(inode),
        )
    }
}

/// Compute compact index region layout from total logical cluster count and ebase.
pub(crate) fn compact_index_layout(totalidx: usize, ebase: usize) -> (usize, usize, usize) {
    let mut compacted_4b_initial = ((32 - ebase % 32) / 4) & 7;
    let compacted_2b;
    if compacted_4b_initial < totalidx {
        compacted_2b = (totalidx - compacted_4b_initial) / 16 * 16;
    } else {
        compacted_4b_initial = 0;
        compacted_2b = 0;
    }
    let compacted_4b_end = totalidx - compacted_4b_initial - compacted_2b;
    (compacted_4b_initial, compacted_2b, compacted_4b_end)
}

/// Compute byte size of the compact index region.
pub(crate) fn compact_index_bytes(totalidx: usize, ebase: usize) -> usize {
    let (c4i, c2b, c4e) = compact_index_layout(totalidx, ebase);
    c4i.div_ceil(2) * 8 + c2b / 16 * 32 + c4e.div_ceil(2) * 8
}

/// Compute BFS-order index sequence.
fn bfs_order(inodes: &[InodeLayout], path_to_idx: &BTreeMap<String, usize>) -> Vec<usize> {
    let mut order = Vec::with_capacity(inodes.len());
    let mut queue = VecDeque::new();

    let Some(&root_idx) = path_to_idx.get("/") else {
        return order;
    };
    order.push(root_idx);
    queue.push_back(root_idx);

    while let Some(dir_idx) = queue.pop_front() {
        let sorted = sorted_children(inodes, dir_idx);
        enqueue_children(&sorted, inodes, path_to_idx, &mut order, &mut queue);
    }
    order
}

/// Return sorted child relative paths for a directory inode.
fn sorted_children(inodes: &[InodeLayout], dir_idx: usize) -> Vec<String> {
    let mut children = inodes[dir_idx].children.clone();
    children.sort_by(|a, b| {
        let na = Path::new(a)
            .file_name()
            .map(|f| f.to_string_lossy())
            .unwrap_or_default();
        let nb = Path::new(b)
            .file_name()
            .map(|f| f.to_string_lossy())
            .unwrap_or_default();
        na.as_ref().cmp(nb.as_ref())
    });
    children
}

/// Push children indices into the visit order and enqueue directories.
fn enqueue_children(
    sorted: &[String],
    inodes: &[InodeLayout],
    path_to_idx: &BTreeMap<String, usize>,
    order: &mut Vec<usize>,
    queue: &mut VecDeque<usize>,
) {
    for child_rel in sorted {
        let Some(&idx) = path_to_idx.get(child_rel.as_str()) else {
            continue;
        };
        order.push(idx);
        if inodes[idx].file_type == EROFS_FT_DIR {
            queue.push_back(idx);
        }
    }
}

/// Compute the datalayout and metadata advance for a directory inode.
fn layout_dir(
    inodes: &mut [InodeLayout],
    i: usize,
    nid: u64,
    slot_offset: usize,
    inode_header: usize,
    path_to_idx: &BTreeMap<String, usize>,
    bs: usize,
) -> usize {
    let dir_entries = build_dir_entry_list(inodes, &inodes[i].children.clone(), path_to_idx, nid);
    let dir_data_size = dir::dir_data_size(&dir_entries);
    let tail_size = dir_data_size % bs;
    let full_blocks = dir_data_size / bs;

    let inode = &mut inodes[i];
    inode.nid = nid;
    inode.size = dir_data_size as u32;

    if dir_data_size > 0 && inline_fits(slot_offset, inode_header, dir_data_size, bs) {
        inode.datalayout = EROFS_INODE_FLAT_INLINE;
        inode.data_blocks = 0;
        padded_slots(inode_header, dir_data_size)
    } else if tail_size > 0 && inline_fits(slot_offset, inode_header, tail_size, bs) {
        inode.datalayout = EROFS_INODE_FLAT_INLINE;
        inode.data_blocks = full_blocks as u32;
        padded_slots(inode_header, tail_size)
    } else {
        inode.datalayout = EROFS_INODE_FLAT_PLAIN;
        inode.data_blocks = dir_data_size.div_ceil(bs) as u32;
        header_only_padded(inode_header)
    }
}

/// Compute the datalayout and metadata advance for a symlink inode.
fn layout_symlink(
    inodes: &mut [InodeLayout],
    i: usize,
    nid: u64,
    slot_offset: usize,
    inode_header: usize,
    bs: usize,
) -> usize {
    let target_len = inodes[i].symlink_target.len();
    let inode = &mut inodes[i];
    inode.nid = nid;
    inode.size = target_len as u32;

    if target_len > 0 && inline_fits(slot_offset, inode_header, target_len, bs) {
        inode.datalayout = EROFS_INODE_FLAT_INLINE;
        padded_slots(inode_header, target_len)
    } else {
        inode.datalayout = EROFS_INODE_FLAT_PLAIN;
        inode.data_blocks = target_len.div_ceil(bs) as u32;
        header_only_padded(inode_header)
    }
}

/// Compute the datalayout and metadata advance for a regular file inode.
fn layout_regular(
    inodes: &mut [InodeLayout],
    i: usize,
    nid: u64,
    slot_offset: usize,
    inode_header: usize,
    bs: usize,
    do_compress: bool,
) -> usize {
    let file_size = inodes[i].size as usize;

    if do_compress
        && file_size > 0
        && let Some(advance) = try_layout_compressed(inodes, i, nid, inode_header)
    {
        return advance;
    }

    let tail_size = file_size % bs;
    let full_blocks = file_size / bs;
    let can_inline_tail = tail_size > 0 && inline_fits(slot_offset, inode_header, tail_size, bs);

    let inode = &mut inodes[i];
    inode.nid = nid;

    if file_size == 0 {
        inode.datalayout = EROFS_INODE_FLAT_PLAIN;
        return header_only_padded(inode_header);
    }

    if can_inline_tail {
        inode.datalayout = EROFS_INODE_FLAT_INLINE;
        inode.data_blocks = full_blocks as u32;
        let inline_len = if full_blocks == 0 {
            file_size
        } else {
            tail_size
        };
        padded_slots(inode_header, inline_len)
    } else {
        inode.datalayout = EROFS_INODE_FLAT_PLAIN;
        inode.data_blocks = file_size.div_ceil(bs) as u32;
        header_only_padded(inode_header)
    }
}

fn inline_fits(slot_offset: usize, inode_header: usize, inline_len: usize, bs: usize) -> bool {
    slot_offset % bs + inode_header + inline_len <= bs
}

/// Try compressing a regular file, returning the meta advance on success.
fn try_layout_compressed(
    inodes: &mut [InodeLayout],
    i: usize,
    nid: u64,
    inode_header: usize,
) -> Option<usize> {
    let file_data = std::fs::read(&inodes[i].path).ok()?;
    let cf = compress::compress_file(&file_data).ok()??;

    let totalidx = compress::lcluster_count(&cf) as usize;
    let pclusters = compress::pcluster_blocks(&cf);

    if pclusters as usize >= totalidx {
        return None;
    }
    let ebase = align8(inode_header) + Z_EROFS_MAP_HEADER_SIZE;
    let index_size = compact_index_bytes(totalidx, ebase);
    let meta_total = ebase + index_size;

    let inode = &mut inodes[i];
    inode.nid = nid;
    inode.datalayout = EROFS_INODE_COMPRESSED_COMPACT;
    inode.data_blocks = pclusters;
    inode.compressed = Some(cf);

    Some(meta_total.div_ceil(SLOT_SIZE) * SLOT_SIZE)
}

/// Compute the datalayout and metadata advance for a special (non-regular) inode.
fn layout_special(inodes: &mut [InodeLayout], i: usize, nid: u64, inode_header: usize) -> usize {
    let inode = &mut inodes[i];
    inode.nid = nid;
    inode.datalayout = EROFS_INODE_FLAT_PLAIN;
    header_only_padded(inode_header)
}

/// Round up to the next multiple of 8.
fn align8(val: usize) -> usize {
    (val + 7) & !7
}

/// Slot-padded size for an inode with optional inline data.
fn padded_slots(inode_header: usize, inline_len: usize) -> usize {
    (inode_header + inline_len).div_ceil(SLOT_SIZE) * SLOT_SIZE
}

/// Slot-padded size for an inode header only (no inline data).
fn header_only_padded(inode_header: usize) -> usize {
    inode_header.div_ceil(SLOT_SIZE) * SLOT_SIZE
}

/// Resolve parent NID by looking up the inode that owns these children.
fn find_parent_nid_from_children(
    all_inodes: &[InodeLayout],
    path_to_idx: &BTreeMap<String, usize>,
    self_nid: u64,
) -> u64 {
    let dir_inode = all_inodes
        .iter()
        .find(|i| i.nid == self_nid && i.file_type == EROFS_FT_DIR);

    let Some(inode) = dir_inode else {
        return self_nid;
    };

    if inode.rel_path == "/" {
        return self_nid;
    }

    let p = parent_rel(&inode.rel_path);
    path_to_idx
        .get(&p)
        .map(|&pidx| all_inodes[pidx].nid)
        .unwrap_or(self_nid)
}

/// Build sorted directory entry list for a given directory inode.
fn build_dir_entry_list(
    all_inodes: &[InodeLayout],
    children: &[String],
    path_to_idx: &BTreeMap<String, usize>,
    self_nid: u64,
) -> Vec<DirEntry> {
    let parent_nid = find_parent_nid_from_children(all_inodes, path_to_idx, self_nid);

    let mut entries = vec![
        DirEntry {
            name: b".".to_vec(),
            nid: self_nid,
            file_type: EROFS_FT_DIR,
        },
        DirEntry {
            name: b"..".to_vec(),
            nid: parent_nid,
            file_type: EROFS_FT_DIR,
        },
    ];

    for child_rel in children {
        if let Some(&idx) = path_to_idx.get(child_rel) {
            let child = &all_inodes[idx];
            let name = std::path::Path::new(child_rel)
                .file_name()
                .map(|n| n.to_string_lossy().as_bytes().to_vec())
                .unwrap_or_default();
            entries.push(DirEntry {
                name,
                nid: child.nid,
                file_type: child.file_type,
            });
        }
    }

    entries.sort_by(|a, b| a.name.cmp(&b.name));
    entries
}

/// Determine how much inline data an inode carries.
fn inline_data_size(inode: &InodeLayout) -> usize {
    if inode.datalayout != EROFS_INODE_FLAT_INLINE {
        return 0;
    }

    let bs = BLOCK_SIZE as usize;
    match inode.file_type {
        EROFS_FT_SYMLINK if inode.data_blocks == 0 => inode.symlink_target.len(),
        EROFS_FT_DIR | EROFS_FT_REG_FILE => {
            let tail = (inode.size as usize) % bs;
            if tail > 0 && inode.data_blocks == 0 {
                inode.size as usize
            } else {
                tail
            }
        }
        _ => 0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layout::types::InodeLayout;

    fn flat_plain_inode(rel_path: &str, file_type: u8) -> InodeLayout {
        InodeLayout {
            path: std::path::PathBuf::new(),
            rel_path: rel_path.to_string(),
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
            inline_data: Vec::new(),
            data_blkaddr: 0,
            data_blocks: 0,
            children: Vec::new(),
            symlink_target: Vec::new(),
            rdev: 0,
            compressed: None,
        }
    }

    #[test]
    fn meta_start_without_compression_is_aligned() {
        // ARRANGE & ACT
        let start = meta_start(false);

        // ASSERT
        assert_eq!(start % SLOT_SIZE, 0);
        assert_eq!(start, META_START.div_ceil(SLOT_SIZE) * SLOT_SIZE);
    }

    #[test]
    fn meta_start_with_compression_is_larger() {
        // ARRANGE & ACT
        let without = meta_start(false);
        let with = meta_start(true);

        // ASSERT
        assert!(with > without);
        assert_eq!(with % SLOT_SIZE, 0);
    }

    #[test]
    fn inline_data_size_flat_plain_returns_zero() {
        // ARRANGE
        let mut inode = flat_plain_inode("/f", EROFS_FT_REG_FILE);
        inode.size = 100;
        inode.datalayout = EROFS_INODE_FLAT_PLAIN;

        // ACT
        let sz = inline_data_size(&inode);

        // ASSERT
        assert_eq!(sz, 0);
    }

    #[test]
    fn inline_data_size_symlink_no_blocks() {
        // ARRANGE
        let mut inode = flat_plain_inode("/l", EROFS_FT_SYMLINK);
        inode.datalayout = EROFS_INODE_FLAT_INLINE;
        inode.symlink_target = b"/target".to_vec();
        inode.data_blocks = 0;

        // ACT
        let sz = inline_data_size(&inode);

        // ASSERT
        assert_eq!(sz, b"/target".len());
    }

    #[test]
    fn inline_data_size_special_file_returns_zero() {
        // ARRANGE
        let mut inode = flat_plain_inode("/dev/null", 0xFF);
        inode.datalayout = EROFS_INODE_FLAT_INLINE;

        // ACT
        let sz = inline_data_size(&inode);

        // ASSERT
        assert_eq!(sz, 0);
    }

    #[test]
    fn inline_data_size_reg_file_entirely_inline() {
        // ARRANGE
        let mut inode = flat_plain_inode("/f", EROFS_FT_REG_FILE);
        inode.datalayout = EROFS_INODE_FLAT_INLINE;
        inode.size = 100;
        inode.data_blocks = 0;

        // ACT
        let sz = inline_data_size(&inode);

        // ASSERT
        assert_eq!(sz, 100);
    }

    #[test]
    fn inline_data_size_reg_file_with_tail() {
        // ARRANGE
        let mut inode = flat_plain_inode("/f", EROFS_FT_REG_FILE);
        inode.datalayout = EROFS_INODE_FLAT_INLINE;
        inode.size = 4196;
        inode.data_blocks = 1;

        // ACT
        let sz = inline_data_size(&inode);

        // ASSERT
        assert_eq!(sz, 100);
    }

    #[test]
    fn bfs_order_empty_path_to_idx_returns_empty() {
        // ARRANGE
        let inodes: Vec<InodeLayout> = Vec::new();
        let path_to_idx: BTreeMap<String, usize> = BTreeMap::new();

        // ACT
        let order = bfs_order(&inodes, &path_to_idx);

        // ASSERT
        assert!(order.is_empty());
    }

    #[test]
    fn assign_nids_special_file_gets_flat_plain() {
        // ARRANGE
        let mut root = flat_plain_inode("/", EROFS_FT_DIR);
        root.children = vec!["/dev".to_string()];
        let mut special = flat_plain_inode("/dev", 3); // EROFS_FT_CHRDEV
        special.rdev = 0x0501;

        let mut inodes = vec![root, special];
        let mut path_to_idx = BTreeMap::new();
        path_to_idx.insert("/".to_string(), 0);
        path_to_idx.insert("/dev".to_string(), 1);

        // ACT
        assign_nids_and_layouts(&mut inodes, &path_to_idx, false);

        // ASSERT
        assert_eq!(inodes[1].datalayout, EROFS_INODE_FLAT_PLAIN);
        assert_ne!(inodes[1].nid, 0);
    }
}
