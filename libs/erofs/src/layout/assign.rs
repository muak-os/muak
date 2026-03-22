//! NID and data layout assignment for each inode in BFS order.

use std::collections::{BTreeMap, VecDeque};
use std::path::Path;

use super::parent_rel;
use super::types::InodeLayout;
use crate::dir::{self, DirEntry, EROFS_FT_DIR, EROFS_FT_REG_FILE, EROFS_FT_SYMLINK};
use crate::inode::{self, COMPACT_INODE_SIZE, EROFS_INODE_FLAT_INLINE, EROFS_INODE_FLAT_PLAIN};
use crate::superblock::EROFS_SUPER_OFFSET;
use crate::{BLOCK_SIZE, SLOT_SIZE};

/// Byte offset at which the inode metadata region begins.
pub(super) const META_START: usize = EROFS_SUPER_OFFSET + 128;

/// Assign NIDs and decide FLAT_INLINE vs FLAT_PLAIN for each inode.
pub fn assign_nids_and_layouts(inodes: &mut [InodeLayout], path_to_idx: &BTreeMap<String, usize>) {
    let bs = BLOCK_SIZE as usize;
    let mut meta_offset = META_START;
    let visit_order = bfs_order(inodes, path_to_idx);

    for i in visit_order {
        let nid = (meta_offset / SLOT_SIZE) as u64;
        let xattr_size = inodes[i].xattr_payload.len();
        let inode_header = COMPACT_INODE_SIZE + xattr_size;

        let advance = match inodes[i].file_type {
            EROFS_FT_DIR => layout_dir(inodes, i, nid, inode_header, path_to_idx, bs),
            EROFS_FT_SYMLINK => layout_symlink(inodes, i, nid, inode_header, bs),
            EROFS_FT_REG_FILE => layout_regular(inodes, i, nid, inode_header, bs),
            _ => layout_special(inodes, i, nid, inode_header),
        };
        meta_offset += advance;
    }
}

/// Assign data block addresses after all NIDs are computed.
pub fn assign_data_block_addrs(inodes: &mut [InodeLayout]) {
    let bs = BLOCK_SIZE as usize;
    let meta_end = compute_meta_end(inodes);
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
pub fn total_image_size(inodes: &[InodeLayout]) -> usize {
    let bs = BLOCK_SIZE as usize;
    let mut max_end = META_START;

    for inode in inodes {
        let slot_end = inode.nid as usize * SLOT_SIZE
            + inode::slot_count(
                COMPACT_INODE_SIZE,
                inode.xattr_payload.len(),
                inline_data_size(inode),
            ) * SLOT_SIZE;
        max_end = max_end.max(slot_end);

        if inode.data_blocks > 0 {
            let data_end = inode.data_blkaddr as usize * bs + inode.data_blocks as usize * bs;
            max_end = max_end.max(data_end);
        }
    }

    max_end.div_ceil(bs) * bs
}

/// Compute the byte offset just past the last inode's meta region.
fn compute_meta_end(inodes: &[InodeLayout]) -> usize {
    inodes
        .iter()
        .map(|inode| {
            inode.nid as usize * SLOT_SIZE
                + inode::slot_count(
                    COMPACT_INODE_SIZE,
                    inode.xattr_payload.len(),
                    inline_data_size(inode),
                ) * SLOT_SIZE
        })
        .max()
        .unwrap_or(META_START)
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

    if dir_data_size <= bs - inode_header {
        inode.datalayout = EROFS_INODE_FLAT_INLINE;
        inode.data_blocks = 0;
        padded_slots(inode_header, dir_data_size)
    } else if tail_size > 0 && inode_header + tail_size <= bs {
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
    inode_header: usize,
    bs: usize,
) -> usize {
    let target_len = inodes[i].symlink_target.len();
    let inode = &mut inodes[i];
    inode.nid = nid;
    inode.size = target_len as u32;

    if inode_header + target_len <= bs {
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
    inode_header: usize,
    bs: usize,
) -> usize {
    let file_size = inodes[i].size as usize;
    let tail_size = file_size % bs;
    let full_blocks = file_size / bs;
    let can_inline_tail = tail_size > 0 && inode_header + tail_size <= bs;

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

/// Compute the datalayout and metadata advance for a special (non-regular) inode.
fn layout_special(inodes: &mut [InodeLayout], i: usize, nid: u64, inode_header: usize) -> usize {
    let inode = &mut inodes[i];
    inode.nid = nid;
    inode.datalayout = EROFS_INODE_FLAT_PLAIN;
    header_only_padded(inode_header)
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
