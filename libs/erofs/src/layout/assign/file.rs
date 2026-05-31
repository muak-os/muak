//! Regular-file, symlink, special-file, and compressed layout decisions.

use super::super::types::InodeLayout;
use super::compact::index_bytes;
use super::util::{align8, header_only_padded, inline_fits, padded_slots, truncate_usize_to_u32};
use crate::checked::align_up;
use crate::compress;
use crate::inode::{
    EROFS_INODE_COMPRESSED_COMPACT, EROFS_INODE_FLAT_INLINE, EROFS_INODE_FLAT_PLAIN,
    Z_EROFS_MAP_HEADER_SIZE,
};
use crate::{Compression, SLOT_SIZE};

pub(super) fn symlink(
    inodes: &mut [InodeLayout],
    i: usize,
    nid: u64,
    slot_offset: usize,
    inode_header: usize,
    bs: usize,
) -> usize {
    let Some(target_len) = inodes.get(i).map(|inode| inode.symlink_target.len()) else {
        return 0;
    };
    let Some(inode) = inodes.get_mut(i) else {
        return 0;
    };
    inode.nid = nid;
    inode.size = truncate_usize_to_u32(target_len);

    if target_len > 0 && inline_fits(slot_offset, inode_header, target_len, bs) {
        inode.datalayout = EROFS_INODE_FLAT_INLINE;
        padded_slots(inode_header, target_len)
    } else {
        inode.datalayout = EROFS_INODE_FLAT_PLAIN;
        inode.data_blocks = truncate_usize_to_u32(target_len.div_ceil(bs));
        header_only_padded(inode_header)
    }
}

pub(super) fn regular(
    inodes: &mut [InodeLayout],
    i: usize,
    nid: u64,
    slot_offset: usize,
    inode_header: usize,
    bs: usize,
    compression: Compression,
) -> usize {
    let Some(file_size) = inodes
        .get(i)
        .map(|inode| usize::try_from(inode.size).unwrap_or_default())
    else {
        return 0;
    };

    if compression.is_enabled()
        && file_size > 0
        && let Some(advance) = try_compressed(inodes, i, nid, inode_header, compression)
    {
        return advance;
    }

    let tail_size = file_size.checked_rem(bs).unwrap_or_default();
    let full_blocks = file_size.checked_div(bs).unwrap_or_default();
    let can_inline_tail = tail_size > 0 && inline_fits(slot_offset, inode_header, tail_size, bs);

    let Some(inode) = inodes.get_mut(i) else {
        return 0;
    };
    inode.nid = nid;

    if file_size == 0 {
        inode.datalayout = EROFS_INODE_FLAT_PLAIN;
        return header_only_padded(inode_header);
    }

    if can_inline_tail {
        inode.datalayout = EROFS_INODE_FLAT_INLINE;
        inode.data_blocks = truncate_usize_to_u32(full_blocks);
        let inline_len = if full_blocks == 0 {
            file_size
        } else {
            tail_size
        };
        padded_slots(inode_header, inline_len)
    } else {
        inode.datalayout = EROFS_INODE_FLAT_PLAIN;
        inode.data_blocks = truncate_usize_to_u32(file_size.div_ceil(bs));
        header_only_padded(inode_header)
    }
}

fn try_compressed(
    inodes: &mut [InodeLayout],
    i: usize,
    nid: u64,
    inode_header: usize,
    compression: Compression,
) -> Option<usize> {
    let Compression::Zstd { level } = compression else {
        return None;
    };
    let file_data = std::fs::read(&inodes.get(i)?.path).ok()?;
    let cf = compress::compress_file(&file_data, level).ok()??;

    if !compress::has_representable_compact_indexes(&cf) {
        return None;
    }

    let totalidx = usize::try_from(compress::lcluster_count(&cf)).ok()?;
    let pclusters = compress::pcluster_blocks(&cf);

    if usize::try_from(pclusters).ok()? >= totalidx {
        return None;
    }
    let ebase = align8(inode_header).saturating_add(Z_EROFS_MAP_HEADER_SIZE);
    let index_size = index_bytes(totalidx, ebase);
    let meta_total = ebase.saturating_add(index_size);

    let inode = inodes.get_mut(i)?;
    inode.nid = nid;
    inode.datalayout = EROFS_INODE_COMPRESSED_COMPACT;
    inode.data_blocks = pclusters;
    inode.compressed = Some(cf);

    Some(align_up(meta_total, SLOT_SIZE).unwrap_or(meta_total))
}

pub(super) fn special(
    inodes: &mut [InodeLayout],
    i: usize,
    nid: u64,
    inode_header: usize,
) -> usize {
    let Some(inode) = inodes.get_mut(i) else {
        return 0;
    };
    inode.nid = nid;
    inode.datalayout = EROFS_INODE_FLAT_PLAIN;
    header_only_padded(inode_header)
}

#[cfg(test)]
mod tests {
    use core::iter::repeat_with;
    use std::os::unix::fs::PermissionsExt as _;

    use super::{regular, special, symlink};
    use crate::Compression;
    use crate::compress::pcluster_blocks;
    use crate::dir::{EROFS_FT_REG_FILE, EROFS_FT_SYMLINK};
    use crate::inode::{
        COMPACT_INODE_SIZE, EROFS_INODE_COMPRESSED_COMPACT, EROFS_INODE_FLAT_INLINE,
        EROFS_INODE_FLAT_PLAIN,
    };
    use crate::layout::{InodeLayout, plan};
    use crate::testutil::{compress_config, test_config};

    #[test]
    fn flat_inline_for_small_files() {
        // ARRANGE
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(dir.path().join("small"), b"hello").expect("write");
        std::fs::set_permissions(
            dir.path().join("small"),
            std::fs::Permissions::from_mode(0o644),
        )
        .expect("chmod");

        let inodes = plan(dir.path(), &test_config(1)).expect("plan");
        let file_inode = inodes
            .iter()
            .find(|inode| inode.rel_path == "/small")
            .expect("found");

        // ACT
        // ASSERT
        assert_eq!(file_inode.datalayout, EROFS_INODE_FLAT_INLINE);
        assert_eq!(file_inode.size, 5);
    }

    #[test]
    fn flat_plain_for_large_files() {
        // ARRANGE
        let dir = tempfile::tempdir().expect("tempdir");
        let data = vec![0_u8; 8192];
        std::fs::write(dir.path().join("large"), &data).expect("write");

        let inodes = plan(dir.path(), &test_config(1)).expect("plan");
        let file_inode = inodes
            .iter()
            .find(|inode| inode.rel_path == "/large")
            .expect("found");

        // ACT
        // ASSERT
        assert_eq!(file_inode.datalayout, EROFS_INODE_FLAT_PLAIN);
        assert_eq!(file_inode.data_blocks, 2);
    }

    #[test]
    fn symlinks_always_inline() {
        // ARRANGE
        let dir = tempfile::tempdir().expect("tempdir");
        std::os::unix::fs::symlink("/target", dir.path().join("link")).expect("symlink");

        let inodes = plan(dir.path(), &test_config(1)).expect("plan");
        let sym = inodes
            .iter()
            .find(|inode| inode.rel_path == "/link")
            .expect("found");

        // ACT
        // ASSERT
        assert_eq!(sym.datalayout, EROFS_INODE_FLAT_INLINE);
        assert_eq!(sym.file_type, EROFS_FT_SYMLINK);
    }

    #[test]
    fn layout_symlink_inline() {
        // ARRANGE
        let dir = tempfile::tempdir().expect("tempdir");
        std::os::unix::fs::symlink("/short", dir.path().join("link")).expect("symlink");

        let inodes = plan(dir.path(), &test_config(1)).expect("plan");
        let link = inodes
            .iter()
            .find(|inode| inode.rel_path == "/link")
            .expect("found");

        // ACT
        // ASSERT
        assert_eq!(link.datalayout, EROFS_INODE_FLAT_INLINE);
        assert_eq!(link.data_blocks, 0);
    }

    #[test]
    fn layout_symlink_flat_plain() {
        // ARRANGE
        let dir = tempfile::tempdir().expect("tempdir");
        let long_target = "/".to_owned() + &"x".repeat(4080);
        std::os::unix::fs::symlink(&long_target, dir.path().join("longlink")).expect("symlink");

        let inodes = plan(dir.path(), &test_config(1)).expect("plan");
        let link = inodes
            .iter()
            .find(|inode| inode.rel_path == "/longlink")
            .expect("found");

        // ACT
        // ASSERT
        assert_eq!(link.datalayout, EROFS_INODE_FLAT_PLAIN);
        assert!(link.data_blocks > 0);
    }

    #[test]
    fn layout_regular_empty_file() {
        // ARRANGE
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(dir.path().join("empty"), b"").expect("write");

        let inodes = plan(dir.path(), &test_config(1)).expect("plan");
        let empty = inodes
            .iter()
            .find(|inode| inode.rel_path == "/empty")
            .expect("found");

        // ACT
        // ASSERT
        assert_eq!(empty.datalayout, EROFS_INODE_FLAT_PLAIN);
        assert_eq!(empty.data_blocks, 0);
        assert_eq!(empty.size, 0);
    }

    #[test]
    fn layout_regular_entirely_inline() {
        // ARRANGE
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(dir.path().join("tiny"), b"hi").expect("write");

        let inodes = plan(dir.path(), &test_config(1)).expect("plan");
        let tiny = inodes
            .iter()
            .find(|inode| inode.rel_path == "/tiny")
            .expect("found");

        // ACT
        // ASSERT
        assert_eq!(tiny.datalayout, EROFS_INODE_FLAT_INLINE);
        assert_eq!(tiny.data_blocks, 0);
    }

    #[test]
    fn layout_regular_inline_with_full_blocks() {
        // ARRANGE
        let dir = tempfile::tempdir().expect("tempdir");
        let data = vec![0_u8; 4100];
        std::fs::write(dir.path().join("partial"), &data).expect("write");

        let inodes = plan(dir.path(), &test_config(1)).expect("plan");
        let partial = inodes
            .iter()
            .find(|inode| inode.rel_path == "/partial")
            .expect("found");

        // ACT
        // ASSERT
        assert_eq!(partial.datalayout, EROFS_INODE_FLAT_INLINE);
        assert!(partial.data_blocks > 0);
    }

    #[test]
    fn compressed_file_gets_compressed_full_layout() {
        // ARRANGE
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(dir.path().join("zeros"), vec![0_u8; 8192]).expect("write");

        let inodes = plan(dir.path(), &compress_config(0)).expect("plan");
        let file = inodes
            .iter()
            .find(|inode| inode.rel_path == "/zeros")
            .expect("found");

        // ACT
        // ASSERT
        assert_eq!(file.datalayout, EROFS_INODE_COMPRESSED_COMPACT);
        assert!(file.compressed.is_some());
        assert!(file.data_blocks > 0);
    }

    #[test]
    fn incompressible_file_falls_back_to_flat() {
        // ARRANGE
        let dir = tempfile::tempdir().expect("tempdir");
        let mut state = 0xDEAD_BEEF_u32;
        let random_data: Vec<u8> = repeat_with(|| {
            state ^= state << 13;
            state ^= state >> 17;
            state ^= state << 5;
            u8::try_from(state & 0xFF).expect("masked byte fits u8")
        })
        .take(8192)
        .collect();
        std::fs::write(dir.path().join("random"), &random_data).expect("write");

        let inodes = plan(dir.path(), &compress_config(0)).expect("plan");
        let file = inodes
            .iter()
            .find(|inode| inode.rel_path == "/random")
            .expect("found");

        // ACT
        // ASSERT
        assert_ne!(file.datalayout, EROFS_INODE_COMPRESSED_COMPACT);
        assert!(file.compressed.is_none());
    }

    #[test]
    fn compressed_empty_file_stays_flat() {
        // ARRANGE
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(dir.path().join("empty"), b"").expect("write");

        let inodes = plan(dir.path(), &compress_config(0)).expect("plan");
        let file = inodes
            .iter()
            .find(|inode| inode.rel_path == "/empty")
            .expect("found");

        // ACT
        // ASSERT
        assert_eq!(file.datalayout, EROFS_INODE_FLAT_PLAIN);
        assert!(file.compressed.is_none());
    }

    #[test]
    fn compressed_small_file_stays_flat_when_no_block_savings() {
        // ARRANGE
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(dir.path().join("small"), vec![0_u8; 100]).expect("write");

        let inodes = plan(dir.path(), &compress_config(0)).expect("plan");
        let file = inodes
            .iter()
            .find(|inode| inode.rel_path == "/small")
            .expect("found");

        // ACT
        // ASSERT
        assert_eq!(file.datalayout, EROFS_INODE_FLAT_INLINE);
        assert!(file.compressed.is_none());
    }

    #[test]
    fn compressed_inode_data_blocks_is_pcluster_count() {
        // ARRANGE
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(dir.path().join("zeros"), vec![0_u8; 8192]).expect("write");

        let inodes = plan(dir.path(), &compress_config(0)).expect("plan");
        let file = inodes
            .iter()
            .find(|inode| inode.rel_path == "/zeros")
            .expect("found");

        let cf = file.compressed.as_ref().expect("compressed");
        let pclusters = pcluster_blocks(cf);
        // ACT
        // ASSERT
        assert_eq!(file.data_blocks, pclusters);
    }

    #[test]
    fn mixed_compressed_and_uncompressed_files() {
        // ARRANGE
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(dir.path().join("compressible"), vec![0_u8; 8192]).expect("write");
        let mut state = 0xCAFE_BABE_u32;
        let random_data: Vec<u8> = repeat_with(|| {
            state ^= state << 13;
            state ^= state >> 17;
            state ^= state << 5;
            u8::try_from(state & 0xFF).expect("masked byte fits u8")
        })
        .take(8192)
        .collect();
        std::fs::write(dir.path().join("random"), &random_data).expect("write");

        let inodes = plan(dir.path(), &compress_config(0)).expect("plan");
        let comp = inodes
            .iter()
            .find(|inode| inode.rel_path == "/compressible")
            .expect("found");
        let rand = inodes
            .iter()
            .find(|inode| inode.rel_path == "/random")
            .expect("found");

        // ACT
        // ASSERT
        assert_eq!(comp.datalayout, EROFS_INODE_COMPRESSED_COMPACT);
        assert!(comp.compressed.is_some());
        assert_ne!(rand.datalayout, EROFS_INODE_COMPRESSED_COMPACT);
        assert!(rand.compressed.is_none());
    }

    #[test]
    fn layout_functions_return_zero_for_missing_inode_index() {
        // ARRANGE
        let mut inodes = vec![InodeLayout {
            path: std::path::PathBuf::new(),
            rel_path: "/".to_owned(),
            nid: 0,
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
            data_blkaddr: 0,
            data_blocks: 0,
            children: Vec::new(),
            symlink_target: Vec::new(),
            rdev: 0,
            compressed: None,
        }];

        let symlink_advance = symlink(&mut inodes, 9, 1, 0, COMPACT_INODE_SIZE, 4096);
        let regular_advance = regular(
            &mut inodes,
            9,
            1,
            0,
            COMPACT_INODE_SIZE,
            4096,
            Compression::None,
        );
        let special_advance = special(&mut inodes, 9, 1, COMPACT_INODE_SIZE);

        // ACT
        // ASSERT
        assert_eq!(symlink_advance, 0);
        assert_eq!(regular_advance, 0);
        assert_eq!(special_advance, 0);
    }
}
