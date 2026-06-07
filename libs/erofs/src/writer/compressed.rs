//! Compressed inode map encoding and pcluster data emission.

use super::dir::align8;
use super::util::{block_offset, block_size_usize, mul, usize_from_u32};
use crate::checked::{add, u16_from_usize, write_byte, write_bytes};
use crate::compress::{self, CompressedFile};
use crate::error::{ErofsError, Result};
use crate::inode::{COMPACT_INODE_SIZE, Z_EROFS_COMPRESSION_ZSTD, Z_EROFS_MAP_HEADER_SIZE};
use crate::layout::{self, InodeLayout};

#[derive(Clone, Copy)]
pub(super) struct LegacyIndexEntry {
    pub(super) clustertype: u8,
    pub(super) clusterofs: u16,
    pub(super) blkaddr: u32,
    pub(super) delta0: u16,
    pub(super) delta1: u16,
}

pub(super) const Z_EROFS_LI_D0_CBLKCNT: u16 = 1 << 11;
pub(super) const Z_EROFS_LCLUSTER_TYPE_PLAIN: u8 = 0;
pub(super) const Z_EROFS_LCLUSTER_TYPE_HEAD1: u8 = 1;
pub(super) const Z_EROFS_LCLUSTER_TYPE_NONHEAD: u8 = 2;

impl LegacyIndexEntry {
    pub(super) const fn plain(clusterofs: u16, blkaddr: u32) -> Self {
        Self {
            clustertype: Z_EROFS_LCLUSTER_TYPE_PLAIN,
            clusterofs,
            blkaddr,
            delta0: 0,
            delta1: 0,
        }
    }

    pub(super) const fn head(clusterofs: u16, blkaddr: u32) -> Self {
        Self {
            clustertype: Z_EROFS_LCLUSTER_TYPE_HEAD1,
            clusterofs,
            blkaddr,
            delta0: 0,
            delta1: 0,
        }
    }

    pub(super) const fn nonhead(delta0: u16, delta1: u16) -> Self {
        Self {
            clustertype: Z_EROFS_LCLUSTER_TYPE_NONHEAD,
            clusterofs: 0,
            blkaddr: 0,
            delta0,
            delta1,
        }
    }

    pub(super) const fn zero() -> Self {
        Self::plain(0, 0)
    }
}

pub(super) struct CompactWriteState {
    pub(super) out_off: usize,
    pub(super) blkaddr_ret: u32,
    pub(super) dummy_head: bool,
    pub(super) blkaddr: u32,
    pub(super) update_blkaddr_in_pack: bool,
}

pub(super) fn write_file(image: &mut [u8], inode: &InodeLayout, slot_offset: usize) -> Result<()> {
    let block_size = block_size_usize();
    let compressed_file = inode
        .compressed
        .as_ref()
        .ok_or(ErofsError::Internal("compressed data present"))?;
    let xattr_size = inode.xattr_payload.len();
    let inode_header_end = add(slot_offset, COMPACT_INODE_SIZE)
        .and_then(|offset| add(offset, xattr_size))
        .ok_or(ErofsError::Internal(
            "compressed inode header offset overflow",
        ))?;

    let map_header_off = align8(inode_header_end);
    write_map_header(image, map_header_off)?;

    let entry_base = add(map_header_off, Z_EROFS_MAP_HEADER_SIZE)
        .ok_or(ErofsError::Internal("compact index base overflow"))?;
    let totalidx = usize_from_u32(compress::lcluster_count(compressed_file));
    let (compact_4b_initial, compact_2b, compact_4b_end) =
        layout::index_layout(totalidx, entry_base);

    if !compress::has_representable_compact_indexes(compressed_file) {
        return Err(ErofsError::Internal(
            "compressed file requires unsupported compact indexes",
        ));
    }

    let entries = build_legacy_index_entries(compressed_file, inode.data_blkaddr)?;
    if entries.len() != totalidx {
        return Err(ErofsError::Internal(
            "compressed index entry count mismatch",
        ));
    }
    let mut state = CompactWriteState {
        out_off: entry_base,
        blkaddr_ret: inode.data_blkaddr,
        dummy_head: false,
        blkaddr: inode.data_blkaddr,
        update_blkaddr_in_pack: false,
    };

    write_indexes(
        image,
        &entries,
        &mut state,
        compact_4b_initial,
        compact_2b,
        compact_4b_end,
    )?;

    let mut block_offset = block_offset(inode.data_blkaddr, block_size, "compressed data")?;
    for pcluster in &compressed_file.pclusters {
        let write_start = add(
            block_offset,
            block_size.saturating_sub(pcluster.compressed_data.len()),
        )
        .ok_or(ErofsError::Internal("compressed write start overflow"))?;
        if !write_bytes(image, write_start, &pcluster.compressed_data) {
            return Err(ErofsError::Internal("compressed data write out of bounds"));
        }
        block_offset = block_offset.saturating_add(block_size);
    }

    Ok(())
}

pub(super) fn build_legacy_index_entries(
    compressed_file: &CompressedFile,
    start_blkaddr: u32,
) -> Result<Vec<LegacyIndexEntry>> {
    let block_size = block_size_usize();
    let totalidx = usize_from_u32(compress::lcluster_count(compressed_file));
    let mut entries = Vec::with_capacity(totalidx);
    let mut cluster_offset = 0_usize;
    let mut blkaddr = start_blkaddr;

    for pcluster in &compressed_file.pclusters {
        let mut local_cluster_offset = cluster_offset;
        let mut remaining_count = pcluster.input_len;
        let mut delta0 = 0_usize;
        let mut delta1 = add(local_cluster_offset, remaining_count)
            .and_then(|value| value.checked_div(block_size))
            .ok_or(ErofsError::Internal("logical cluster size overflow"))?;
        let head_cluster_offset = u16_from_usize(local_cluster_offset)
            .ok_or(ErofsError::Internal("head cluster offset does not fit u16"))?;

        if delta1 == 0 {
            entries.push(LegacyIndexEntry::head(head_cluster_offset, blkaddr));
            cluster_offset = 0_usize;
            blkaddr = blkaddr.saturating_add(1);
            continue;
        }

        while add(local_cluster_offset, remaining_count).is_some_and(|value| value >= block_size) {
            entries.push(match delta0 {
                0 => LegacyIndexEntry::head(head_cluster_offset, blkaddr),
                1 => LegacyIndexEntry::nonhead(
                    Z_EROFS_LI_D0_CBLKCNT | 1,
                    u16_from_usize(delta1)
                        .ok_or(ErofsError::Internal("delta1 does not fit u16"))?,
                ),
                _ => LegacyIndexEntry::nonhead(
                    u16_from_usize(delta0.min(usize::from(Z_EROFS_LI_D0_CBLKCNT - 1)))
                        .ok_or(ErofsError::Internal("delta0 does not fit u16"))?,
                    u16_from_usize(delta1)
                        .ok_or(ErofsError::Internal("delta1 does not fit u16"))?,
                ),
            });

            remaining_count =
                remaining_count.saturating_sub(block_size.saturating_sub(local_cluster_offset));
            local_cluster_offset = 0_usize;
            delta0 = delta0.saturating_add(1);
            delta1 = delta1.saturating_sub(1);
        }

        cluster_offset = add(local_cluster_offset, remaining_count)
            .ok_or(ErofsError::Internal("cluster offset overflow"))?;
        blkaddr = blkaddr.saturating_add(1);
    }

    if cluster_offset != 0 {
        entries.push(LegacyIndexEntry::plain(
            u16_from_usize(cluster_offset)
                .ok_or(ErofsError::Internal("tail cluster offset does not fit u16"))?,
            0,
        ));
    }

    Ok(entries)
}

pub(super) fn write_indexes(
    image: &mut [u8],
    entries: &[LegacyIndexEntry],
    state: &mut CompactWriteState,
    c4i: usize,
    c2b: usize,
    c4e: usize,
) -> Result<()> {
    let mut entry_index = 0_usize;

    let mut remaining_4b_initial = c4i;
    while remaining_4b_initial > 0 {
        let pack = two_entry_pack(entries, entry_index)?;
        write_pack(image, state, &pack, 4, false, true)?;
        entry_index = entry_index.saturating_add(2);
        remaining_4b_initial = remaining_4b_initial.saturating_sub(2);
    }

    let mut remaining_2b = c2b;
    while remaining_2b >= 16 {
        let pack = sixteen_entry_pack(entries, entry_index)?;
        write_pack(image, state, &pack, 2, false, true)?;
        entry_index = entry_index.saturating_add(16);
        remaining_2b = remaining_2b.saturating_sub(16);
    }

    let mut remaining_4b_end = c4e;
    while remaining_4b_end > 1 {
        let pack = two_entry_pack(entries, entry_index)?;
        write_pack(image, state, &pack, 4, false, true)?;
        entry_index = entry_index.saturating_add(2);
        remaining_4b_end = remaining_4b_end.saturating_sub(2);
    }

    if remaining_4b_end == 1 {
        let first_entry = *entries
            .get(entry_index)
            .ok_or(ErofsError::Internal("missing final compact index entry"))?;
        let pack = [first_entry, LegacyIndexEntry::zero()];
        write_pack(image, state, &pack, 4, true, true)?;
        entry_index = entry_index.saturating_add(1);
    }

    debug_assert_eq!(
        entry_index,
        entries.len(),
        "all compact index entries should be written"
    );

    Ok(())
}

pub(super) fn write_pack(
    image: &mut [u8],
    state: &mut CompactWriteState,
    pack: &[LegacyIndexEntry],
    destsize: usize,
    final_pack: bool,
    update_blkaddr: bool,
) -> Result<()> {
    const LOBITS: u32 = 12;
    let value_count = pack.len();
    let total_bits = mul(value_count, destsize)
        .and_then(|value| mul(value, 8))
        .ok_or(ErofsError::Internal("compact pack bit length overflow"))?;
    let encodebits = total_bits
        .saturating_sub(32)
        .checked_div(value_count)
        .ok_or(ErofsError::Internal("compact pack encoding width overflow"))?;
    let mut out = [0_u8; 32];
    let out_len =
        mul(destsize, value_count).ok_or(ErofsError::Internal("compact pack length overflow"))?;

    let mut bit_position = 0_usize;
    state.blkaddr = state.blkaddr_ret;
    state.update_blkaddr_in_pack = update_blkaddr;

    for (index, entry) in pack.iter().enumerate() {
        let offset = if entry.clustertype == Z_EROFS_LCLUSTER_TYPE_NONHEAD {
            nonhead_offset(entry, index, value_count, state)
        } else {
            head_offset(entry, index, value_count, final_pack, state)
        };

        let encoded = (u32::from(entry.clustertype) << LOBITS) | u32::from(offset);
        pack_bits_le(
            out.get_mut(..out_len)
                .ok_or(ErofsError::Internal("compact pack buffer out of bounds"))?,
            bit_position,
            encoded,
            encodebits,
        )?;
        bit_position = bit_position.saturating_add(encodebits);
    }

    debug_assert_eq!(
        out_len.saturating_mul(8),
        bit_position.saturating_add(32),
        "compact pack bit layout should reserve a 32-bit trailer"
    );
    let trailer_offset = out_len.saturating_sub(4);
    let trailer = out
        .get_mut(trailer_offset..trailer_offset.saturating_add(4))
        .ok_or(ErofsError::Internal("compact pack trailer out of bounds"))?;
    trailer.copy_from_slice(&state.blkaddr_ret.to_le_bytes());
    state.blkaddr_ret = state.blkaddr;
    let pack_bytes = out
        .get(..out_len)
        .ok_or(ErofsError::Internal("compact pack slice out of bounds"))?;
    if !write_bytes(image, state.out_off, pack_bytes) {
        return Err(ErofsError::Internal("compact pack write out of bounds"));
    }
    state.out_off = state.out_off.saturating_add(out_len);

    Ok(())
}

pub(super) fn pack_bits_le(
    buf: &mut [u8],
    bit_offset: usize,
    value: u32,
    nbits: usize,
) -> Result<()> {
    for bit_index in 0..nbits {
        let shift = u32::try_from(bit_index)
            .ok()
            .ok_or(ErofsError::Internal("bit index does not fit u32"))?;
        if value & 1_u32.checked_shl(shift).unwrap_or(0) != 0 {
            let position = bit_offset.saturating_add(bit_index);
            let byte_index = position >> 3;
            let bit_shift = u32::try_from(position & 7)
                .ok()
                .ok_or(ErofsError::Internal("bit position does not fit u32"))?;
            let mask = 1_u8
                .checked_shl(bit_shift)
                .ok_or(ErofsError::Internal("bit mask shift overflow"))?;
            let byte = buf
                .get_mut(byte_index)
                .ok_or(ErofsError::Internal("packed bit write out of bounds"))?;
            *byte |= mask;
        }
    }

    Ok(())
}

pub(super) fn head_offset(
    entry: &LegacyIndexEntry,
    index: usize,
    value_count: usize,
    final_pack: bool,
    state: &mut CompactWriteState,
) -> u16 {
    if state.dummy_head {
        state.blkaddr = state.blkaddr.saturating_add(1);
        if state.update_blkaddr_in_pack {
            state.blkaddr_ret = state.blkaddr;
        }
    }
    state.dummy_head = true;
    state.update_blkaddr_in_pack = false;
    debug_assert!(
        entry.blkaddr == state.blkaddr || index.saturating_add(1) == value_count || final_pack,
        "head entry block address should match the compact pack state"
    );
    debug_assert!(
        entry.blkaddr == state.blkaddr || entry.blkaddr == 0,
        "head entry block address should be current or zero"
    );
    entry.clusterofs
}

pub(super) fn nonhead_offset(
    entry: &LegacyIndexEntry,
    index: usize,
    value_count: usize,
    state: &mut CompactWriteState,
) -> u16 {
    if entry.delta0 & Z_EROFS_LI_D0_CBLKCNT != 0 {
        state.blkaddr = state
            .blkaddr
            .saturating_add(u32::from(entry.delta0 & !Z_EROFS_LI_D0_CBLKCNT));
        state.dummy_head = false;
        entry.delta0
    } else if index.saturating_add(1) == value_count {
        entry.delta1.min(Z_EROFS_LI_D0_CBLKCNT - 1)
    } else {
        entry.delta0
    }
}

pub(super) fn write_map_header(image: &mut [u8], offset: usize) -> Result<()> {
    if !write_bytes(image, offset, &0_u32.to_le_bytes()) {
        return Err(ErofsError::Internal("map header prefix out of bounds"));
    }
    let h_advise: u16 = 0x0007;
    if !write_bytes(image, offset.saturating_add(4), &h_advise.to_le_bytes()) {
        return Err(ErofsError::Internal("map header advice out of bounds"));
    }
    let algorithmtype: u8 = Z_EROFS_COMPRESSION_ZSTD;
    if !write_byte(image, offset.saturating_add(6), algorithmtype) {
        return Err(ErofsError::Internal("map header algorithm out of bounds"));
    }
    if !write_byte(image, offset.saturating_add(7), 0) {
        return Err(ErofsError::Internal("map header clusterbits out of bounds"));
    }

    Ok(())
}

pub(super) fn two_entry_pack(
    entries: &[LegacyIndexEntry],
    index: usize,
) -> Result<[LegacyIndexEntry; 2]> {
    let first_entry = *entries
        .get(index)
        .ok_or(ErofsError::Internal("missing compact index entry"))?;
    let second_entry = *entries
        .get(index.saturating_add(1))
        .ok_or(ErofsError::Internal("missing compact index entry"))?;
    Ok([first_entry, second_entry])
}

pub(super) fn sixteen_entry_pack(
    entries: &[LegacyIndexEntry],
    index: usize,
) -> Result<[LegacyIndexEntry; 16]> {
    let entry_slice = entries
        .get(index..index.saturating_add(16))
        .ok_or(ErofsError::Internal("missing 16-entry compact index pack"))?;
    let mut pack = [LegacyIndexEntry::zero(); 16];
    pack.copy_from_slice(entry_slice);
    Ok(pack)
}

#[cfg(test)]
mod tests {
    use zstd::bulk::decompress;

    use super::{
        CompactWriteState, LegacyIndexEntry, Z_EROFS_LCLUSTER_TYPE_HEAD1,
        Z_EROFS_LCLUSTER_TYPE_NONHEAD, Z_EROFS_LCLUSTER_TYPE_PLAIN, Z_EROFS_LI_D0_CBLKCNT,
        build_legacy_index_entries, head_offset, nonhead_offset, pack_bits_le, sixteen_entry_pack,
        two_entry_pack, write_indexes, write_map_header, write_pack,
    };
    use crate::SLOT_SIZE;
    use crate::compress::{self, CompressedFile};
    use crate::error::ErofsError;
    use crate::inode::COMPACT_INODE_SIZE;
    use crate::layout;
    use crate::layout::collect::FilesystemTreeSource;
    use crate::testutil::compress_config;
    use crate::writer::write_image;

    fn compressed_file(data_len: usize) -> CompressedFile {
        let data = vec![0_u8; data_len];
        compress::compress_file(&data, 3)
            .expect("compress")
            .expect("compressed file")
    }

    #[test]
    fn write_compressed_map_header_zstd() {
        // ARRANGE
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(dir.path().join("zeros"), vec![0_u8; 8192]).expect("write");
        let cfg = compress_config(0);

        let planned = layout::plan(&FilesystemTreeSource::new(dir.path()), &cfg).expect("plan");
        let image = write_image(&planned, &cfg).expect("write");

        let file = planned
            .inodes
            .iter()
            .find(|inode| inode.rel_path == "/zeros")
            .expect("found");
        let slot_off = usize::try_from(file.nid).expect("nid fits usize") * SLOT_SIZE;
        let xattr_size = file.xattr_payload.len();
        let map_off = super::align8(slot_off + COMPACT_INODE_SIZE + xattr_size);
        // ACT
        // ASSERT
        assert_eq!(*image.get(map_off + 6).expect("algorithm byte"), 3);
        assert_eq!(*image.get(map_off + 7).expect("clusterbits byte"), 0);
    }

    #[test]
    fn write_compressed_data_at_blkaddr() {
        // ARRANGE
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(dir.path().join("zeros"), vec![0_u8; 8192]).expect("write");
        let cfg = compress_config(0);

        let planned = layout::plan(&FilesystemTreeSource::new(dir.path()), &cfg).expect("plan");
        let image = write_image(&planned, &cfg).expect("write");

        let file = planned
            .inodes
            .iter()
            .find(|inode| inode.rel_path == "/zeros")
            .expect("found");
        let cf = file.compressed.as_ref().expect("compressed");
        let mut blk_off = usize::try_from(file.data_blkaddr).expect("blkaddr fits usize") * 4096;
        for pcluster in &cf.pclusters {
            let write_start = blk_off + 4096 - pcluster.compressed_data.len();
            // ACT
            // ASSERT
            assert_eq!(
                image
                    .get(write_start..write_start + pcluster.compressed_data.len())
                    .expect("compressed data bytes"),
                pcluster.compressed_data.as_slice()
            );
            blk_off += 4096;
        }
    }

    #[test]
    fn write_compressed_compact_index_entries() {
        // ARRANGE
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(dir.path().join("zeros"), vec![0_u8; 8192]).expect("write");
        let cfg = compress_config(0);

        let planned = layout::plan(&FilesystemTreeSource::new(dir.path()), &cfg).expect("plan");
        let image = write_image(&planned, &cfg).expect("write");

        let file = planned
            .inodes
            .iter()
            .find(|inode| inode.rel_path == "/zeros")
            .expect("found");
        let cf = file.compressed.as_ref().expect("compressed");
        let slot_off = usize::try_from(file.nid).expect("nid fits usize") * SLOT_SIZE;
        let map_off = super::align8(slot_off + COMPACT_INODE_SIZE + file.xattr_payload.len());
        let h_advise = u16::from_le_bytes(
            image
                .get(map_off + 4..map_off + 6)
                .expect("header advice bytes")
                .try_into()
                .expect("2b"),
        );
        // ACT
        // ASSERT
        assert_eq!(h_advise, 0x0007);
        let ebase = map_off + 8;
        let totalidx = usize::try_from(compress::lcluster_count(cf)).expect("totalidx fits usize");
        let (c4i, c2b, c4e) = layout::index_layout(totalidx, ebase);
        assert_eq!(c4i + c2b + c4e, totalidx);
    }

    #[test]
    fn build_legacy_index_entries_tracks_clusterofs_and_local_d1() {
        // ARRANGE
        let cf = compressed_file(17_000);

        let entries = build_legacy_index_entries(&cf, 123).expect("entries");

        // ACT
        // ASSERT
        assert_eq!(entries.len(), 5);
        let first = entries.first().expect("first entry");
        assert_eq!(first.clustertype, Z_EROFS_LCLUSTER_TYPE_HEAD1);
        assert_eq!(first.clusterofs, 0);
        assert_eq!(first.blkaddr, 123);
        let middle = entries.get(1..entries.len() - 1).expect("middle entries");
        assert!(middle.iter().all(|entry| {
            matches!(
                entry.clustertype,
                Z_EROFS_LCLUSTER_TYPE_HEAD1 | Z_EROFS_LCLUSTER_TYPE_NONHEAD
            )
        }));
        assert!(
            middle
                .iter()
                .any(|entry| entry.clustertype == Z_EROFS_LCLUSTER_TYPE_NONHEAD)
        );
        let last = entries.get(4).expect("last entry");
        assert_eq!(last.clustertype, Z_EROFS_LCLUSTER_TYPE_PLAIN);
        assert_eq!(last.clusterofs, 616);
    }

    #[test]
    fn write_compacted_pack_encodes_head_clusterofs() {
        // ARRANGE
        let mut image = vec![0_u8; 4096];
        let mut state = CompactWriteState {
            out_off: 512,
            blkaddr_ret: 200,
            dummy_head: false,
            blkaddr: 200,
            update_blkaddr_in_pack: false,
        };
        let pack = [LegacyIndexEntry::head(904, 200), LegacyIndexEntry::zero()];

        write_pack(&mut image, &mut state, &pack, 4, true, true).expect("pack");

        let encoded_first = u16::from_le_bytes(
            image
                .get(512..514)
                .expect("first entry bytes")
                .try_into()
                .expect("2b"),
        );
        let expected = (u16::from(Z_EROFS_LCLUSTER_TYPE_HEAD1) << 12) | 0x0388;
        // ACT
        // ASSERT
        assert_eq!(encoded_first, expected);

        let trailer_blkaddr = u32::from_le_bytes(
            image
                .get(516..520)
                .expect("trailer block address bytes")
                .try_into()
                .expect("4b"),
        );
        assert_eq!(trailer_blkaddr, 200);
        assert_eq!(state.blkaddr_ret, 201);
    }

    #[test]
    fn write_compressed_decompresses_correctly() {
        // ARRANGE
        let dir = tempfile::tempdir().expect("tempdir");
        let original = vec![0_u8; 8192];
        std::fs::write(dir.path().join("zeros"), &original).expect("write");
        let cfg = compress_config(0);

        let planned = layout::plan(&FilesystemTreeSource::new(dir.path()), &cfg).expect("plan");
        let image = write_image(&planned, &cfg).expect("write");

        let file = planned
            .inodes
            .iter()
            .find(|inode| inode.rel_path == "/zeros")
            .expect("found");
        let cf = file.compressed.as_ref().expect("compressed");
        let mut blk_off = usize::try_from(file.data_blkaddr).expect("blkaddr fits usize") * 4096;
        let mut input_off = 0_usize;
        for pcluster in &cf.pclusters {
            let write_start = blk_off + 4096 - pcluster.compressed_data.len();
            let compressed_data = image
                .get(write_start..write_start + pcluster.compressed_data.len())
                .expect("compressed data bytes");
            let decompressed = decompress(compressed_data, pcluster.input_len).expect("decompress");
            // ACT
            // ASSERT
            assert_eq!(
                decompressed,
                original
                    .get(input_off..input_off + pcluster.input_len)
                    .expect("original input bytes")
            );
            input_off += pcluster.input_len;
            blk_off += 4096;
        }
    }

    #[test]
    fn helper_packs_and_map_header_report_out_of_bounds() {
        // ARRANGE
        let entries = [LegacyIndexEntry::head(0, 0)];
        let mut image = [0_u8; 4];

        let two_pack = two_entry_pack(&entries, 0);
        let sixteen_pack = sixteen_entry_pack(&entries, 0);
        let header_write = write_map_header(&mut image, 0);

        // ACT
        // ASSERT
        assert!(two_pack.is_err());
        assert!(sixteen_pack.is_err());
        assert!(matches!(
            header_write,
            Err(ErofsError::Internal("map header advice out of bounds"))
        ));
    }

    #[test]
    fn compact_index_helpers_cover_final_pack_and_tail_cases() {
        // ARRANGE
        let mut image = vec![0_u8; 64];
        let entries = vec![LegacyIndexEntry::head(0, 7)];
        let mut state = CompactWriteState {
            out_off: 0,
            blkaddr_ret: 7,
            dummy_head: false,
            blkaddr: 7,
            update_blkaddr_in_pack: false,
        };
        let nonhead_entry = LegacyIndexEntry::nonhead(2, Z_EROFS_LI_D0_CBLKCNT + 5);

        let write_result = write_indexes(&mut image, &entries, &mut state, 0, 0, 1);
        let tail_offset = nonhead_offset(&nonhead_entry, 0, 1, &mut state);

        // ACT
        // ASSERT
        write_result.expect("write indexes");
        assert_eq!(tail_offset, Z_EROFS_LI_D0_CBLKCNT - 1);
    }

    #[test]
    fn pack_bits_and_head_offset_cover_error_paths() {
        // ARRANGE
        let mut buf = [0_u8; 0];
        let mut state = CompactWriteState {
            out_off: 0,
            blkaddr_ret: 1,
            dummy_head: true,
            blkaddr: 1,
            update_blkaddr_in_pack: true,
        };

        let pack_result = pack_bits_le(&mut buf, 0, 1, 1);
        let head = head_offset(&LegacyIndexEntry::head(3, 2), 0, 1, true, &mut state);

        // ACT
        // ASSERT
        assert!(pack_result.is_err());
        assert_eq!(head, 3);
        assert_eq!(state.blkaddr_ret, 2);
    }

    #[test]
    fn compact_index_helpers_cover_remaining_pack_paths() {
        // ARRANGE
        let mut image = vec![0_u8; 128];
        let entries = build_legacy_index_entries(&compressed_file(18 * 4096), 0).expect("entries");
        let mut state = CompactWriteState {
            out_off: 0,
            blkaddr_ret: 0,
            dummy_head: false,
            blkaddr: 0,
            update_blkaddr_in_pack: false,
        };
        let mut write_image_buf = [0_u8; 4];
        let mut tiny_state = CompactWriteState {
            out_off: 0,
            blkaddr_ret: 0,
            dummy_head: false,
            blkaddr: 0,
            update_blkaddr_in_pack: false,
        };

        let write_indexes = write_indexes(&mut image, &entries, &mut state, 0, 16, 2);
        tiny_state.out_off = 2;
        let write_error = write_pack(
            &mut write_image_buf,
            &mut tiny_state,
            &[LegacyIndexEntry::head(0, 0), LegacyIndexEntry::head(0, 1)],
            4,
            false,
            true,
        );

        // ACT
        // ASSERT
        write_indexes.expect("write indexes");
        assert!(entries.len() >= 18);
        assert!(matches!(
            write_error,
            Err(ErofsError::Internal("compact pack write out of bounds"))
        ));
    }

    #[test]
    fn write_map_header_reports_suffix_out_of_bounds() {
        // ARRANGE
        let mut advice_image = [0_u8; 5];
        let mut algorithm_image = [0_u8; 6];
        let mut clusterbits_image = [0_u8; 7];

        let advice_error = write_map_header(&mut advice_image, 0);
        let algorithm_error = write_map_header(&mut algorithm_image, 0);
        let clusterbits_error = write_map_header(&mut clusterbits_image, 0);

        // ACT
        // ASSERT
        assert!(matches!(
            advice_error,
            Err(ErofsError::Internal("map header advice out of bounds"))
        ));
        assert!(matches!(
            algorithm_error,
            Err(ErofsError::Internal("map header algorithm out of bounds"))
        ));
        assert!(matches!(
            clusterbits_error,
            Err(ErofsError::Internal("map header clusterbits out of bounds"))
        ));
    }
}
