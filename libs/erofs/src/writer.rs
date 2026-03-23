//! EROFS image writer producing raw image bytes.

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use crate::dir::{self, DirEntry, EROFS_FT_DIR, EROFS_FT_REG_FILE, EROFS_FT_SYMLINK};
use crate::error::Result;
use crate::inode::{
    self, COMPACT_INODE_SIZE, CompactInodeParams, EROFS_INODE_FLAT_INLINE,
    Z_EROFS_COMPRESSION_ZSTD, Z_EROFS_MAP_HEADER_SIZE,
};
use crate::layout::{self, InodeLayout};
use crate::superblock::{self, SuperblockParams};
use crate::{BLOCK_SIZE, SLOT_SIZE};

/// Build a complete EROFS image from the planned layout.
pub fn write_image(inodes: &[InodeLayout], config: &crate::MkfsConfig<'_>) -> Result<Vec<u8>> {
    let bs = BLOCK_SIZE as usize;
    let total_size = layout::total_image_size(inodes, config.compress);
    let mut image = vec![0u8; total_size];

    let path_to_idx: BTreeMap<String, usize> = inodes
        .iter()
        .enumerate()
        .map(|(i, n)| (n.rel_path.clone(), i))
        .collect();

    for inode in inodes {
        let slot_offset = inode.nid as usize * SLOT_SIZE;
        let xattr_size = inode.xattr_payload.len();
        let inode_header_end = slot_offset + COMPACT_INODE_SIZE + xattr_size;

        write_inode_header(&mut image, inode, slot_offset);

        match inode.file_type {
            EROFS_FT_DIR => {
                write_dir_data(
                    &mut image,
                    inode,
                    inodes,
                    &path_to_idx,
                    inode_header_end,
                    bs,
                );
            }
            EROFS_FT_SYMLINK => write_symlink_data(&mut image, inode, inode_header_end, bs),
            EROFS_FT_REG_FILE if inode.compressed.is_some() => {
                write_compressed_file_data(&mut image, inode, slot_offset)?;
            }
            EROFS_FT_REG_FILE if inode.size > 0 => {
                write_file_data(&mut image, inode, inode_header_end, bs)?;
            }
            _ => {}
        }
    }

    let has_compressed = config.compress;
    let root_nid = inodes.first().map_or(0, |i| i.nid as u16);
    let blocks = (total_size / bs) as u32;

    superblock::write_superblock(
        &mut image,
        &SuperblockParams {
            root_nid,
            inos: inodes.len() as u64,
            epoch: config.source_date_epoch,
            blocks,
            uuid: config.uuid,
            has_compression: has_compressed,
        },
    );
    superblock::write_checksum(&mut image);

    Ok(image)
}

/// Write the 32-byte compact inode header and xattr payload into the image.
fn write_inode_header(image: &mut [u8], inode: &InodeLayout, slot_offset: usize) {
    let i_u = if inode.compressed.is_some() {
        inode.data_blocks
    } else if inode.file_type != EROFS_FT_DIR
        && inode.file_type != EROFS_FT_REG_FILE
        && inode.file_type != EROFS_FT_SYMLINK
    {
        inode.rdev
    } else if inode.data_blocks > 0 {
        inode.data_blkaddr
    } else if inode.file_type == EROFS_FT_REG_FILE && inode.size == 0 {
        0
    } else {
        u32::MAX
    };

    inode::write_compact_inode(
        &mut image[slot_offset..slot_offset + COMPACT_INODE_SIZE],
        &CompactInodeParams {
            datalayout: inode.datalayout,
            xattr_icount: inode.xattr_icount,
            mode: inode.mode,
            nlink: inode.nlink,
            size: inode.size,
            startblk_or_rdev: i_u,
            ino: inode.ino,
            uid: inode.uid,
            gid: inode.gid,
            reserved2: 0,
        },
    );

    if !inode.xattr_payload.is_empty() {
        let xattr_start = slot_offset + COMPACT_INODE_SIZE;
        let xattr_end = xattr_start + inode.xattr_payload.len();
        image[xattr_start..xattr_end].copy_from_slice(&inode.xattr_payload);
    }
}

/// Write inline and/or block data for a combined inline+block layout.
fn write_inline_data(
    image: &mut [u8],
    data: &[u8],
    data_blocks: u32,
    data_blkaddr: u32,
    data_size: usize,
    inode_header_end: usize,
    bs: usize,
) {
    let full_block_bytes = data_blocks as usize * bs;
    if data_blocks > 0 {
        let data_start = data_blkaddr as usize * bs;
        image[data_start..data_start + full_block_bytes].copy_from_slice(&data[..full_block_bytes]);
    }
    let tail_len = data_size - full_block_bytes;
    if tail_len > 0 {
        image[inode_header_end..inode_header_end + tail_len]
            .copy_from_slice(&data[full_block_bytes..full_block_bytes + tail_len]);
    }
}

/// Write block-only data for FLAT_PLAIN layout.
fn write_plain_data(image: &mut [u8], data: &[u8], data_blkaddr: u32, bs: usize) {
    let data_start = data_blkaddr as usize * bs;
    image[data_start..data_start + data.len()].copy_from_slice(data);
}

fn write_dir_data(
    image: &mut [u8],
    inode: &InodeLayout,
    all_inodes: &[InodeLayout],
    path_to_idx: &BTreeMap<String, usize>,
    inode_header_end: usize,
    bs: usize,
) {
    let parent_nid = find_parent_nid(inode, all_inodes, path_to_idx);
    let dir_entries = build_sorted_dir_entries(inode, all_inodes, path_to_idx, parent_nid);
    let dir_data = dir::serialize_dir_entries(&dir_entries);

    if inode.datalayout == EROFS_INODE_FLAT_INLINE {
        write_inline_data(
            image,
            &dir_data,
            inode.data_blocks,
            inode.data_blkaddr,
            inode.size as usize,
            inode_header_end,
            bs,
        );
    } else {
        write_plain_data(image, &dir_data, inode.data_blkaddr, bs);
    }
}

fn write_symlink_data(image: &mut [u8], inode: &InodeLayout, inode_header_end: usize, bs: usize) {
    if inode.datalayout == EROFS_INODE_FLAT_INLINE {
        write_inline_data(
            image,
            &inode.symlink_target,
            inode.data_blocks,
            inode.data_blkaddr,
            inode.symlink_target.len(),
            inode_header_end,
            bs,
        );
    } else {
        let data_start = inode.data_blkaddr as usize * bs;
        image[data_start..data_start + inode.symlink_target.len()]
            .copy_from_slice(&inode.symlink_target);
    }
}

fn write_file_data(
    image: &mut [u8],
    inode: &InodeLayout,
    inode_header_end: usize,
    bs: usize,
) -> Result<()> {
    let file_data = fs::read(&inode.path)?;

    if inode.datalayout == EROFS_INODE_FLAT_INLINE {
        write_inline_data(
            image,
            &file_data,
            inode.data_blocks,
            inode.data_blkaddr,
            file_data.len(),
            inode_header_end,
            bs,
        );
    } else {
        let data_start = inode.data_blkaddr as usize * bs;
        image[data_start..data_start + file_data.len()].copy_from_slice(&file_data);
    }
    Ok(())
}

/// Write compressed file data: map header, compact indexes, and pcluster blocks.
fn write_compressed_file_data(
    image: &mut [u8],
    inode: &InodeLayout,
    slot_offset: usize,
) -> Result<()> {
    let bs = BLOCK_SIZE as usize;
    let cf = inode.compressed.as_ref().expect("compressed data present");
    let xattr_size = inode.xattr_payload.len();
    let inode_header_end = slot_offset + COMPACT_INODE_SIZE + xattr_size;

    let map_header_off = align8(inode_header_end);
    write_z_erofs_map_header(image, map_header_off);

    let ebase = map_header_off + Z_EROFS_MAP_HEADER_SIZE;
    let totalidx = crate::compress::lcluster_count(cf) as usize;
    let (c4i, c2b, c4e) = layout::compact_index_layout(totalidx, ebase);

    let entries = build_legacy_index_entries(cf, inode.data_blkaddr);
    debug_assert_eq!(entries.len(), totalidx);
    let mut st = CompactWriteState {
        out_off: ebase,
        blkaddr_ret: inode.data_blkaddr,
        dummy_head: false,
    };

    write_compact_indexes(image, &entries, &mut st, c4i, c2b, c4e);

    let mut blk_off = inode.data_blkaddr as usize * bs;
    for pc in &cf.pclusters {
        let write_start = blk_off + bs - pc.compressed_data.len();
        image[write_start..write_start + pc.compressed_data.len()]
            .copy_from_slice(&pc.compressed_data);
        blk_off += bs;
    }

    Ok(())
}

/// Legacy lcluster index vector used as compact conversion input.
#[derive(Clone, Copy)]
struct LegacyIndexEntry {
    clustertype: u8,
    clusterofs: u16,
    blkaddr: u32,
    delta0: u16,
    delta1: u16,
}

const Z_EROFS_LI_D0_CBLKCNT: u16 = 1 << 11;
const Z_EROFS_LCLUSTER_TYPE_PLAIN: u8 = 0;
const Z_EROFS_LCLUSTER_TYPE_HEAD1: u8 = 1;
const Z_EROFS_LCLUSTER_TYPE_NONHEAD: u8 = 2;

impl LegacyIndexEntry {
    const fn plain(clusterofs: u16, blkaddr: u32) -> Self {
        Self {
            clustertype: Z_EROFS_LCLUSTER_TYPE_PLAIN,
            clusterofs,
            blkaddr,
            delta0: 0,
            delta1: 0,
        }
    }

    const fn head(clusterofs: u16, blkaddr: u32) -> Self {
        Self {
            clustertype: Z_EROFS_LCLUSTER_TYPE_HEAD1,
            clusterofs,
            blkaddr,
            delta0: 0,
            delta1: 0,
        }
    }

    const fn nonhead(delta0: u16, delta1: u16) -> Self {
        Self {
            clustertype: Z_EROFS_LCLUSTER_TYPE_NONHEAD,
            clusterofs: 0,
            blkaddr: 0,
            delta0,
            delta1,
        }
    }

    const fn zero() -> Self {
        Self::plain(0, 0)
    }
}

/// Build full-index semantics from compressed pclusters.
fn build_legacy_index_entries(
    cf: &crate::compress::CompressedFile,
    start_blkaddr: u32,
) -> Vec<LegacyIndexEntry> {
    let bs = BLOCK_SIZE as usize;
    let totalidx = crate::compress::lcluster_count(cf) as usize;
    let mut entries = Vec::with_capacity(totalidx);
    let mut clusterofs = 0usize;
    let mut blkaddr = start_blkaddr;

    for pc in &cf.pclusters {
        let mut local_clusterofs = clusterofs;
        let mut count = pc.input_len;
        let mut d0 = 0usize;
        let mut d1 = (local_clusterofs + count) / bs;
        let head_clusterofs = local_clusterofs as u16;

        if d1 == 0 {
            entries.push(LegacyIndexEntry::head(head_clusterofs, blkaddr));
            clusterofs = 0;
            blkaddr += 1;
            continue;
        }

        while local_clusterofs + count >= bs {
            if d0 == 0 {
                entries.push(LegacyIndexEntry::head(head_clusterofs, blkaddr));
            } else if d0 == 1 {
                entries.push(LegacyIndexEntry::nonhead(
                    Z_EROFS_LI_D0_CBLKCNT | 1,
                    d1 as u16,
                ));
            } else {
                let encoded_d0 = d0.min((Z_EROFS_LI_D0_CBLKCNT - 1) as usize) as u16;
                entries.push(LegacyIndexEntry::nonhead(encoded_d0, d1 as u16));
            }

            count -= bs - local_clusterofs;
            local_clusterofs = 0;
            d0 += 1;
            d1 -= 1;
        }

        clusterofs = local_clusterofs + count;
        blkaddr += 1;
    }

    if clusterofs != 0 {
        entries.push(LegacyIndexEntry::plain(clusterofs as u16, 0));
    }

    entries
}

struct CompactWriteState {
    out_off: usize,
    blkaddr_ret: u32,
    dummy_head: bool,
}

/// Convert full-index vectors into compact packs.
fn write_compact_indexes(
    image: &mut [u8],
    entries: &[LegacyIndexEntry],
    st: &mut CompactWriteState,
    c4i: usize,
    c2b: usize,
    c4e: usize,
) {
    let mut idx = 0usize;

    let mut remaining_4b_initial = c4i;
    while remaining_4b_initial > 0 {
        let pack = [entries[idx], entries[idx + 1]];
        write_compacted_pack(image, st, &pack, 4, false, true);
        idx += 2;
        remaining_4b_initial -= 2;
    }

    let mut remaining_2b = c2b;
    while remaining_2b >= 16 {
        let mut pack = [LegacyIndexEntry::zero(); 16];
        pack.copy_from_slice(&entries[idx..idx + 16]);
        write_compacted_pack(image, st, &pack, 2, false, true);
        idx += 16;
        remaining_2b -= 16;
    }

    let mut remaining_4b_end = c4e;
    while remaining_4b_end > 1 {
        let pack = [entries[idx], entries[idx + 1]];
        write_compacted_pack(image, st, &pack, 4, false, true);
        idx += 2;
        remaining_4b_end -= 2;
    }

    if remaining_4b_end == 1 {
        let pack = [entries[idx], LegacyIndexEntry::zero()];
        write_compacted_pack(image, st, &pack, 4, true, true);
        idx += 1;
    }

    debug_assert_eq!(idx, entries.len());
}

/// Write one compact index pack using upstream-compatible semantics.
fn write_compacted_pack(
    image: &mut [u8],
    st: &mut CompactWriteState,
    pack: &[LegacyIndexEntry],
    destsize: usize,
    final_pack: bool,
    update_blkaddr: bool,
) {
    const LOBITS: u32 = 12;
    let vcnt = pack.len();
    let encodebits = (vcnt * destsize * 8 - 32) / vcnt;
    let mut out = [0u8; 32];
    let out_len = destsize * vcnt;

    let mut bit_pos = 0usize;
    let mut blkaddr = st.blkaddr_ret;
    let mut update_blkaddr_in_pack = update_blkaddr;

    for (i, e) in pack.iter().enumerate() {
        let offset = if e.clustertype == Z_EROFS_LCLUSTER_TYPE_NONHEAD {
            if e.delta0 & Z_EROFS_LI_D0_CBLKCNT != 0 {
                let cblks = e.delta0 & !Z_EROFS_LI_D0_CBLKCNT;
                blkaddr += cblks as u32;
                st.dummy_head = false;
                e.delta0
            } else if i + 1 == vcnt {
                e.delta1.min(Z_EROFS_LI_D0_CBLKCNT - 1)
            } else {
                e.delta0
            }
        } else {
            if st.dummy_head {
                blkaddr += 1;
                if update_blkaddr_in_pack {
                    st.blkaddr_ret = blkaddr;
                }
            }
            st.dummy_head = true;
            update_blkaddr_in_pack = false;

            if e.blkaddr != blkaddr {
                debug_assert!(i + 1 == vcnt || final_pack);
                debug_assert_eq!(e.blkaddr, 0);
            }

            e.clusterofs
        };

        let encoded = ((e.clustertype as u32) << LOBITS) | u32::from(offset);
        pack_bits_le(&mut out[..out_len], bit_pos, encoded, encodebits);
        bit_pos += encodebits;
    }

    debug_assert_eq!(out_len * 8, bit_pos + 32);
    out[out_len - 4..out_len].copy_from_slice(&st.blkaddr_ret.to_le_bytes());
    st.blkaddr_ret = blkaddr;
    image[st.out_off..st.out_off + out_len].copy_from_slice(&out[..out_len]);
    st.out_off += out_len;
}

/// Pack `nbits` of `value` into a byte buffer at the given bit offset (LE).
fn pack_bits_le(buf: &mut [u8], bit_offset: usize, value: u32, nbits: usize) {
    for i in 0..nbits {
        if value & (1 << i) != 0 {
            let pos = bit_offset + i;
            buf[pos / 8] |= 1 << (pos % 8);
        }
    }
}

/// Write the 8-byte z_erofs_map_header for zstd big-pcluster compact mode.
fn write_z_erofs_map_header(image: &mut [u8], offset: usize) {
    image[offset..offset + 4].copy_from_slice(&0u32.to_le_bytes());
    let h_advise: u16 = 0x0007;
    image[offset + 4..offset + 6].copy_from_slice(&h_advise.to_le_bytes());
    let algorithmtype: u8 = Z_EROFS_COMPRESSION_ZSTD;
    image[offset + 6] = algorithmtype;
    image[offset + 7] = 0;
}

fn find_parent_nid(
    inode: &InodeLayout,
    all_inodes: &[InodeLayout],
    path_to_idx: &BTreeMap<String, usize>,
) -> u64 {
    if inode.rel_path == "/" {
        return inode.nid;
    }
    let parent_rel = Path::new(&inode.rel_path)
        .parent()
        .map(|p| {
            let s = p.to_string_lossy().to_string();
            if s.is_empty() { "/".to_string() } else { s }
        })
        .unwrap_or_else(|| "/".to_string());

    path_to_idx
        .get(&parent_rel)
        .map(|&idx| all_inodes[idx].nid)
        .unwrap_or(0)
}

/// Round up to the next multiple of 8.
fn align8(val: usize) -> usize {
    (val + 7) & !7
}

fn build_sorted_dir_entries(
    inode: &InodeLayout,
    all_inodes: &[InodeLayout],
    path_to_idx: &BTreeMap<String, usize>,
    parent_nid: u64,
) -> Vec<DirEntry> {
    let mut entries = vec![
        DirEntry {
            name: b".".to_vec(),
            nid: inode.nid,
            file_type: EROFS_FT_DIR,
        },
        DirEntry {
            name: b"..".to_vec(),
            nid: parent_nid,
            file_type: EROFS_FT_DIR,
        },
    ];

    for child_rel in &inode.children {
        if let Some(&idx) = path_to_idx.get(child_rel) {
            let child = &all_inodes[idx];
            let name = Path::new(child_rel)
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::MkfsConfig;
    use crate::inode::{
        EROFS_INODE_COMPRESSED_COMPACT, EROFS_INODE_FLAT_INLINE, EROFS_INODE_FLAT_PLAIN,
    };
    use crate::superblock::{EROFS_SUPER_MAGIC_V1, EROFS_SUPER_OFFSET};

    fn test_config(epoch: u64) -> MkfsConfig<'static> {
        MkfsConfig {
            source_date_epoch: epoch,
            file_contexts: None,
            uuid: [0; 16],
            force_uid: None,
            force_gid: None,
            compress: false,
        }
    }

    #[test]
    fn write_image_empty_file_has_zero_startblk() {
        // ARRANGE
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(dir.path().join("empty"), b"").expect("write");
        let cfg = test_config(1);

        // ACT
        let inodes = layout::plan(dir.path(), &cfg).expect("plan");
        let image = write_image(&inodes, &cfg).expect("write");

        // ASSERT
        let empty = inodes
            .iter()
            .find(|i| i.rel_path == "/empty")
            .expect("found");
        let slot_offset = empty.nid as usize * SLOT_SIZE;
        let startblk = u32::from_le_bytes(
            image[slot_offset + 0x10..slot_offset + 0x14]
                .try_into()
                .expect("4 bytes"),
        );
        assert_eq!(startblk, 0);
    }

    #[test]
    fn find_parent_nid_for_root() {
        // ARRANGE
        let dir = tempfile::tempdir().expect("tempdir");
        let cfg = test_config(1);

        // ACT
        let inodes = layout::plan(dir.path(), &cfg).expect("plan");
        let _image = write_image(&inodes, &cfg).expect("write");

        // ASSERT
        let root = &inodes[0];
        assert_eq!(root.rel_path, "/");
    }

    #[test]
    fn superblock_at_correct_offset() {
        // ARRANGE
        let dir = tempfile::tempdir().expect("tempdir");
        let cfg = test_config(1);

        // ACT
        let inodes = layout::plan(dir.path(), &cfg).expect("plan");
        let image = write_image(&inodes, &cfg).expect("write");

        // ASSERT
        let magic = u32::from_le_bytes(
            image[EROFS_SUPER_OFFSET..EROFS_SUPER_OFFSET + 4]
                .try_into()
                .expect("4 bytes"),
        );
        assert_eq!(magic, EROFS_SUPER_MAGIC_V1);
    }

    #[test]
    fn root_nid_matches_root_dir() {
        // ARRANGE
        let dir = tempfile::tempdir().expect("tempdir");
        let cfg = test_config(1);

        // ACT
        let inodes = layout::plan(dir.path(), &cfg).expect("plan");
        let image = write_image(&inodes, &cfg).expect("write");

        // ASSERT
        let root_nid = u16::from_le_bytes(
            image[EROFS_SUPER_OFFSET + 0x0E..EROFS_SUPER_OFFSET + 0x10]
                .try_into()
                .expect("2 bytes"),
        );
        assert_eq!(root_nid, inodes[0].nid as u16);
    }

    #[test]
    fn root_nid_is_36_in_image() {
        // ARRANGE
        let dir = tempfile::tempdir().expect("tempdir");
        let cfg = test_config(1);

        // ACT
        let inodes = layout::plan(dir.path(), &cfg).expect("plan");
        let image = write_image(&inodes, &cfg).expect("write");

        // ASSERT
        let root_nid = u16::from_le_bytes(
            image[EROFS_SUPER_OFFSET + 0x0E..EROFS_SUPER_OFFSET + 0x10]
                .try_into()
                .expect("2 bytes"),
        );
        assert_eq!(root_nid, 36);
    }

    #[test]
    fn reproducible_output() {
        // ARRANGE
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(dir.path().join("a"), b"aaa").expect("write");
        std::fs::write(dir.path().join("b"), b"bbb").expect("write");
        let uuid = [1u8; 16];
        let cfg = MkfsConfig {
            source_date_epoch: 1000,
            file_contexts: None,
            uuid,
            force_uid: None,
            force_gid: None,
            compress: false,
        };

        // ACT
        let inodes1 = layout::plan(dir.path(), &cfg).expect("plan");
        let image1 = write_image(&inodes1, &cfg).expect("write");
        let inodes2 = layout::plan(dir.path(), &cfg).expect("plan");
        let image2 = write_image(&inodes2, &cfg).expect("write");

        // ASSERT
        assert_eq!(image1, image2);
    }

    #[test]
    fn compact_inode_at_correct_offset() {
        // ARRANGE
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(dir.path().join("test"), b"data").expect("write");
        let cfg = test_config(0);

        // ACT
        let inodes = layout::plan(dir.path(), &cfg).expect("plan");
        let image = write_image(&inodes, &cfg).expect("write");

        // ASSERT
        let root_offset = 36 * SLOT_SIZE;
        let i_format = u16::from_le_bytes(
            image[root_offset..root_offset + 2]
                .try_into()
                .expect("2 bytes"),
        );
        assert_eq!(i_format & 0x01, 0, "compact inode (bit 0 = 0)");
    }

    #[test]
    fn write_image_with_inline_file() {
        // ARRANGE
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(dir.path().join("small"), b"hello").expect("write");
        let cfg = test_config(1);

        // ACT
        let inodes = layout::plan(dir.path(), &cfg).expect("plan");
        let image = write_image(&inodes, &cfg).expect("write");

        // ASSERT
        let file_inode = inodes
            .iter()
            .find(|i| i.rel_path == "/small")
            .expect("found");
        assert_eq!(file_inode.datalayout, EROFS_INODE_FLAT_INLINE);
        assert!(image.len() >= 4096);
    }

    #[test]
    fn write_dir_data_inline() {
        // ARRANGE
        let dir = tempfile::tempdir().expect("tempdir");
        for i in 0..5u8 {
            std::fs::write(dir.path().join(format!("f{i}")), [i]).expect("write");
        }
        let cfg = test_config(1);

        // ACT
        let inodes = layout::plan(dir.path(), &cfg).expect("plan");
        let image = write_image(&inodes, &cfg).expect("write");

        // ASSERT
        let root = &inodes[0];
        assert_eq!(root.datalayout, EROFS_INODE_FLAT_INLINE);
        assert!(image.len() >= 4096);
    }

    #[test]
    fn write_dir_data_plain() {
        // ARRANGE
        let dir = tempfile::tempdir().expect("tempdir");
        for i in 0u16..339 {
            let name = format!("file_{i:03}.txt");
            std::fs::write(dir.path().join(&name), [i as u8]).expect("write");
        }
        let cfg = test_config(1);

        // ACT
        let inodes = layout::plan(dir.path(), &cfg).expect("plan");

        // ASSERT
        let root = &inodes[0];
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
        let inodes = layout::plan(dir.path(), &cfg).expect("plan");
        let image = write_image(&inodes, &cfg).expect("write");

        // ASSERT
        let link = inodes
            .iter()
            .find(|i| i.rel_path == "/link")
            .expect("found");
        assert_eq!(link.datalayout, EROFS_INODE_FLAT_INLINE);
        assert!(image.len() >= 4096);
    }

    #[test]
    fn write_symlink_data_plain() {
        // ARRANGE
        let dir = tempfile::tempdir().expect("tempdir");
        let long_target = "/".to_string() + &"x".repeat(4080);
        std::os::unix::fs::symlink(&long_target, dir.path().join("longlink")).expect("symlink");
        let cfg = test_config(1);

        // ACT
        let inodes = layout::plan(dir.path(), &cfg).expect("plan");
        let _image = write_image(&inodes, &cfg).expect("write");

        // ASSERT
        let link = inodes
            .iter()
            .find(|i| i.rel_path == "/longlink")
            .expect("found");
        assert_eq!(link.datalayout, EROFS_INODE_FLAT_PLAIN);
        assert!(link.data_blocks > 0);
    }

    #[test]
    fn write_file_data_with_inline_tail() {
        // ARRANGE
        let dir = tempfile::tempdir().expect("tempdir");
        let data = vec![0u8; 4100];
        std::fs::write(dir.path().join("partial"), &data).expect("write");
        let cfg = test_config(1);

        // ACT
        let inodes = layout::plan(dir.path(), &cfg).expect("plan");
        let _image = write_image(&inodes, &cfg).expect("write");

        // ASSERT
        let file = inodes
            .iter()
            .find(|i| i.rel_path == "/partial")
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
        let inodes = layout::plan(dir.path(), &cfg).expect("plan");
        let _image = write_image(&inodes, &cfg).expect("write");

        // ASSERT
        let file = inodes
            .iter()
            .find(|i| i.rel_path == "/tiny")
            .expect("found");
        assert_eq!(file.data_blocks, 0);
    }

    #[test]
    fn find_parent_nid_for_nested_file() {
        // ARRANGE
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::create_dir(dir.path().join("subdir")).expect("mkdir");
        std::fs::write(dir.path().join("subdir/file.txt"), b"content").expect("write");
        let cfg = test_config(1);

        // ACT
        let inodes = layout::plan(dir.path(), &cfg).expect("plan");
        let _image = write_image(&inodes, &cfg).expect("write");

        // ASSERT
        let subdir = inodes
            .iter()
            .find(|i| i.rel_path == "/subdir")
            .expect("found");
        assert_eq!(subdir.nid, 39);
    }

    #[test]
    fn build_sorted_dir_entries() {
        // ARRANGE
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(dir.path().join("z"), b"z").expect("write");
        std::fs::write(dir.path().join("a"), b"a").expect("write");
        std::fs::write(dir.path().join("m"), b"m").expect("write");
        let cfg = test_config(1);

        // ACT
        let inodes = layout::plan(dir.path(), &cfg).expect("plan");
        let image = write_image(&inodes, &cfg).expect("write");

        // ASSERT
        assert!(image.len() >= 4096);
    }

    #[test]
    fn write_image_with_selinux_xattr() {
        // ARRANGE
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(dir.path().join("f"), b"x").expect("write");
        let fc =
            crate::FileContexts::from_reader("/.*    system_u:object_r:file_t:s0\n".as_bytes())
                .expect("fc");
        let cfg = MkfsConfig {
            source_date_epoch: 0,
            file_contexts: Some(&fc),
            uuid: [0; 16],
            force_uid: None,
            force_gid: None,
            compress: false,
        };

        // ACT
        let inodes = layout::plan(dir.path(), &cfg).expect("plan");
        let _image = write_image(&inodes, &cfg).expect("write");

        // ASSERT
        let file = inodes.iter().find(|i| i.rel_path == "/f").expect("found");
        assert!(!file.xattr_payload.is_empty());
    }

    fn compress_config(epoch: u64) -> MkfsConfig<'static> {
        MkfsConfig {
            source_date_epoch: epoch,
            file_contexts: None,
            uuid: [0; 16],
            force_uid: None,
            force_gid: None,
            compress: true,
        }
    }

    #[test]
    fn write_compressed_image_valid_size() {
        // ARRANGE
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(dir.path().join("zeros"), vec![0u8; 8192]).expect("write");
        let cfg = compress_config(0);

        // ACT
        let inodes = layout::plan(dir.path(), &cfg).expect("plan");
        let image = write_image(&inodes, &cfg).expect("write");

        // ASSERT
        assert_eq!(image.len() % 4096, 0);
        assert!(image.len() >= 4096);
    }

    #[test]
    fn write_compressed_inode_has_compressed_compact_format() {
        // ARRANGE
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(dir.path().join("zeros"), vec![0u8; 8192]).expect("write");
        let cfg = compress_config(0);

        // ACT
        let inodes = layout::plan(dir.path(), &cfg).expect("plan");
        let image = write_image(&inodes, &cfg).expect("write");

        // ASSERT
        let file = inodes
            .iter()
            .find(|i| i.rel_path == "/zeros")
            .expect("found");
        let slot_off = file.nid as usize * SLOT_SIZE;
        let i_format = u16::from_le_bytes(image[slot_off..slot_off + 2].try_into().expect("2b"));
        let datalayout = (i_format >> 1) & 0x07;
        assert_eq!(datalayout, EROFS_INODE_COMPRESSED_COMPACT);
    }

    #[test]
    fn write_compressed_inode_i_u_is_pcluster_blocks() {
        // ARRANGE
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(dir.path().join("zeros"), vec![0u8; 8192]).expect("write");
        let cfg = compress_config(0);

        // ACT
        let inodes = layout::plan(dir.path(), &cfg).expect("plan");
        let image = write_image(&inodes, &cfg).expect("write");

        // ASSERT
        let file = inodes
            .iter()
            .find(|i| i.rel_path == "/zeros")
            .expect("found");
        let slot_off = file.nid as usize * SLOT_SIZE;
        let i_u = u32::from_le_bytes(
            image[slot_off + 0x10..slot_off + 0x14]
                .try_into()
                .expect("4b"),
        );
        let cf = file.compressed.as_ref().expect("compressed");
        assert_eq!(i_u, crate::compress::pcluster_blocks(cf));
    }

    #[test]
    fn write_compressed_map_header_zstd() {
        // ARRANGE
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(dir.path().join("zeros"), vec![0u8; 8192]).expect("write");
        let cfg = compress_config(0);

        // ACT
        let inodes = layout::plan(dir.path(), &cfg).expect("plan");
        let image = write_image(&inodes, &cfg).expect("write");

        // ASSERT
        let file = inodes
            .iter()
            .find(|i| i.rel_path == "/zeros")
            .expect("found");
        let slot_off = file.nid as usize * SLOT_SIZE;
        let xattr_size = file.xattr_payload.len();
        let map_off = align8(slot_off + COMPACT_INODE_SIZE + xattr_size);
        assert_eq!(image[map_off + 6], 3, "h_algorithmtype = zstd(3)");
        assert_eq!(image[map_off + 7], 0, "h_clusterbits = 0");
    }

    #[test]
    fn write_compressed_data_at_blkaddr() {
        // ARRANGE
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(dir.path().join("zeros"), vec![0u8; 8192]).expect("write");
        let cfg = compress_config(0);

        // ACT
        let inodes = layout::plan(dir.path(), &cfg).expect("plan");
        let image = write_image(&inodes, &cfg).expect("write");

        // ASSERT: each pcluster's data is right-aligned in its block
        let file = inodes
            .iter()
            .find(|i| i.rel_path == "/zeros")
            .expect("found");
        let cf = file.compressed.as_ref().expect("compressed");
        let mut blk_off = file.data_blkaddr as usize * 4096;
        for pc in &cf.pclusters {
            let write_start = blk_off + 4096 - pc.compressed_data.len();
            assert_eq!(
                &image[write_start..write_start + pc.compressed_data.len()],
                pc.compressed_data.as_slice()
            );
            blk_off += 4096;
        }
    }

    #[test]
    fn write_compressed_superblock_compr_cfgs() {
        // ARRANGE
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(dir.path().join("zeros"), vec![0u8; 4096]).expect("write");
        let cfg = compress_config(0);

        // ACT
        let inodes = layout::plan(dir.path(), &cfg).expect("plan");
        let image = write_image(&inodes, &cfg).expect("write");

        // ASSERT
        let cfg_off = EROFS_SUPER_OFFSET + 128;
        let cfg_size = u16::from_le_bytes(image[cfg_off..cfg_off + 2].try_into().expect("2b"));
        assert_eq!(cfg_size, 6, "compr cfg size");
        assert_eq!(image[cfg_off + 2], 0, "format = 0");
        assert_eq!(image[cfg_off + 3], 5, "windowlog = 5");
    }

    #[test]
    fn write_compressed_compact_index_entries() {
        // ARRANGE
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(dir.path().join("zeros"), vec![0u8; 8192]).expect("write");
        let cfg = compress_config(0);

        // ACT
        let inodes = layout::plan(dir.path(), &cfg).expect("plan");
        let image = write_image(&inodes, &cfg).expect("write");

        // ASSERT
        let file = inodes
            .iter()
            .find(|i| i.rel_path == "/zeros")
            .expect("found");
        let cf = file.compressed.as_ref().expect("compressed");
        let slot_off = file.nid as usize * SLOT_SIZE;
        let map_off = align8(slot_off + COMPACT_INODE_SIZE + file.xattr_payload.len());
        let h_advise = u16::from_le_bytes(image[map_off + 4..map_off + 6].try_into().expect("2b"));
        assert_eq!(h_advise, 0x0007, "Z_EROFS_ADVISE_COMPACTED_2B|BIG_PCLUSTER");
        let ebase = map_off + 8;
        let totalidx = crate::compress::lcluster_count(cf) as usize;
        let (c4i, c2b, c4e) = layout::compact_index_layout(totalidx, ebase);
        assert!(c4i + c2b + c4e == totalidx, "zone counts sum to totalidx");
    }

    #[test]
    fn build_legacy_index_entries_tracks_clusterofs_and_local_d1() {
        // ARRANGE
        let cf = crate::compress::CompressedFile {
            pclusters: vec![
                crate::compress::Pcluster {
                    compressed_data: vec![0u8; 64],
                    input_len: 5000,
                },
                crate::compress::Pcluster {
                    compressed_data: vec![0u8; 64],
                    input_len: 12_000,
                },
            ],
            original_size: 17_000,
        };

        // ACT
        let entries = build_legacy_index_entries(&cf, 123);

        // ASSERT
        assert_eq!(entries.len(), 5);

        assert_eq!(entries[0].clustertype, Z_EROFS_LCLUSTER_TYPE_HEAD1);
        assert_eq!(entries[0].clusterofs, 0);
        assert_eq!(entries[0].blkaddr, 123);

        assert_eq!(entries[1].clustertype, Z_EROFS_LCLUSTER_TYPE_HEAD1);
        assert_eq!(entries[1].clusterofs, 904);
        assert_eq!(entries[1].blkaddr, 124);

        assert_eq!(entries[2].clustertype, Z_EROFS_LCLUSTER_TYPE_NONHEAD);
        assert_eq!(entries[2].delta0, Z_EROFS_LI_D0_CBLKCNT | 1);
        assert_eq!(entries[2].delta1, 2);

        assert_eq!(entries[3].clustertype, Z_EROFS_LCLUSTER_TYPE_NONHEAD);
        assert_eq!(entries[3].delta0, 2);
        assert_eq!(entries[3].delta1, 1);

        assert_eq!(entries[4].clustertype, Z_EROFS_LCLUSTER_TYPE_PLAIN);
        assert_eq!(entries[4].clusterofs, 616);
    }

    #[test]
    fn write_compacted_pack_encodes_head_clusterofs() {
        // ARRANGE
        let mut image = vec![0u8; 4096];
        let mut st = CompactWriteState {
            out_off: 512,
            blkaddr_ret: 200,
            dummy_head: false,
        };
        let pack = [LegacyIndexEntry::head(904, 200), LegacyIndexEntry::zero()];

        // ACT
        write_compacted_pack(&mut image, &mut st, &pack, 4, true, true);

        // ASSERT
        let encoded_first = u16::from_le_bytes(image[512..514].try_into().expect("2b"));
        let expected = ((Z_EROFS_LCLUSTER_TYPE_HEAD1 as u16) << 12) | 904;
        assert_eq!(encoded_first, expected);

        let trailer_blkaddr = u32::from_le_bytes(image[516..520].try_into().expect("4b"));
        assert_eq!(trailer_blkaddr, 200);
        assert_eq!(st.blkaddr_ret, 201);
    }

    #[test]
    fn write_compressed_decompresses_correctly() {
        // ARRANGE
        let dir = tempfile::tempdir().expect("tempdir");
        let original = vec![0u8; 8192];
        std::fs::write(dir.path().join("zeros"), &original).expect("write");
        let cfg = compress_config(0);

        // ACT
        let inodes = layout::plan(dir.path(), &cfg).expect("plan");
        let image = write_image(&inodes, &cfg).expect("write");

        // ASSERT: each pcluster decompresses to its original slice
        let file = inodes
            .iter()
            .find(|i| i.rel_path == "/zeros")
            .expect("found");
        let cf = file.compressed.as_ref().expect("compressed");
        let mut blk_off = file.data_blkaddr as usize * 4096;
        let mut input_off = 0usize;
        for pc in &cf.pclusters {
            let write_start = blk_off + 4096 - pc.compressed_data.len();
            let compressed_data = &image[write_start..write_start + pc.compressed_data.len()];
            let decompressed =
                zstd::bulk::decompress(compressed_data, pc.input_len).expect("decompress");
            assert_eq!(decompressed, &original[input_off..input_off + pc.input_len]);
            input_off += pc.input_len;
            blk_off += 4096;
        }
    }

    #[test]
    fn write_file_data_plain_layout() {
        // ARRANGE: a file that is exactly 4096 bytes has no tail, so FLAT_PLAIN.
        let dir = tempfile::tempdir().expect("tempdir");
        let data = vec![0xABu8; 4096];
        std::fs::write(dir.path().join("full"), &data).expect("write");
        let cfg = test_config(1);

        // ACT
        let inodes = layout::plan(dir.path(), &cfg).expect("plan");
        let image = write_image(&inodes, &cfg).expect("write");

        // ASSERT
        let file = inodes
            .iter()
            .find(|i| i.rel_path == "/full")
            .expect("found");
        assert_eq!(file.datalayout, EROFS_INODE_FLAT_PLAIN);
        let data_start = file.data_blkaddr as usize * 4096;
        assert_eq!(&image[data_start..data_start + 4096], data.as_slice());
    }

    #[test]
    fn write_inode_header_rdev_for_special_file() {
        // ARRANGE
        let inode = InodeLayout {
            path: std::path::PathBuf::new(),
            rel_path: "/dev/null".to_string(),
            nid: 36,
            ino: 0,
            mode: 0o020666,
            uid: 0,
            gid: 0,
            mtime: 0,
            mtime_nsec: 0,
            nlink: 1,
            file_type: 3, // EROFS_FT_CHRDEV
            size: 0,
            datalayout: EROFS_INODE_FLAT_PLAIN,
            xattr_payload: Vec::new(),
            xattr_icount: 0,
            inline_data: Vec::new(),
            data_blkaddr: 0,
            data_blocks: 0,
            children: Vec::new(),
            symlink_target: Vec::new(),
            rdev: 0x0501,
            compressed: None,
        };
        let mut image = vec![0u8; 8192];
        let slot_offset = inode.nid as usize * SLOT_SIZE;

        // ACT
        write_inode_header(&mut image, &inode, slot_offset);

        // ASSERT
        let stored = u32::from_le_bytes(
            image[slot_offset + 0x10..slot_offset + 0x14]
                .try_into()
                .expect("4 bytes"),
        );
        assert_eq!(stored, 0x0501);
    }
}
