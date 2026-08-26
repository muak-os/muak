//! Regular-file, symlink, special-file, and compressed layout decisions.

use super::super::types::InodeLayout;
use super::compact::index_bytes;
use super::sizes::{align8, header_only_padded, inline_fits, padded_slots, truncate_usize_to_u32};
use crate::checked::align_up;
use crate::compress;
use crate::error::{ErofsError, Result};
use crate::inode::{
    EROFS_INODE_COMPRESSED_COMPACT, EROFS_INODE_FLAT_INLINE, EROFS_INODE_FLAT_PLAIN,
    Z_EROFS_MAP_HEADER_SIZE,
};
use crate::source::SizedFile;
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

#[expect(
    clippy::too_many_arguments,
    reason = "all parameters are required for layout decision"
)]
pub(super) fn regular(
    inodes: &mut [InodeLayout],
    i: usize,
    nid: u64,
    slot_offset: usize,
    inode_header: usize,
    bs: usize,
    compression: Compression,
    files: &mut [SizedFile<'_>],
) -> Result<usize> {
    let Some(file_size) = inodes
        .get(i)
        .map(|inode| usize::try_from(inode.size).unwrap_or_default())
    else {
        return Ok(0);
    };

    let compressed_advance = match (file_size > 0, compression.level()) {
        (true, Some(level)) => {
            compressed_regular(inodes, i, nid, inode_header, files, file_size, level)?
        }
        _ => None,
    };
    if let Some(advance) = compressed_advance {
        return Ok(advance);
    }

    let full_blocks = file_size.checked_div(bs).unwrap_or_default();
    let can_inline = full_blocks == 0 && inline_fits(slot_offset, inode_header, file_size, bs);

    let Some(inode) = inodes.get_mut(i) else {
        return Ok(0);
    };
    inode.nid = nid;

    if file_size == 0 {
        inode.datalayout = EROFS_INODE_FLAT_PLAIN;
        return Ok(header_only_padded(inode_header));
    }

    if can_inline {
        inode.datalayout = EROFS_INODE_FLAT_INLINE;
        Ok(padded_slots(inode_header, file_size))
    } else {
        inode.datalayout = EROFS_INODE_FLAT_PLAIN;
        inode.data_blocks = truncate_usize_to_u32(file_size.div_ceil(bs));

        Ok(header_only_padded(inode_header))
    }
}

fn compressed_regular(
    inodes: &mut [InodeLayout],
    i: usize,
    nid: u64,
    inode_header: usize,
    files: &mut [SizedFile<'_>],
    file_size: usize,
    level: i32,
) -> Result<Option<usize>> {
    let Some(rel_path) = inodes.get(i).map(|inode| inode.rel_path.clone()) else {
        return Ok(None);
    };
    let sized = files
        .get_mut(i)
        .ok_or(ErofsError::Internal("file index out of bounds"))?;
    let Some(cf) = compress::compress_file(sized.reader, file_size, &rel_path, level)? else {
        return Ok(None);
    };

    if !compress::has_representable_compact_indexes(&cf) {
        return Ok(None);
    }

    let Some(totalidx) = usize::try_from(compress::lcluster_count(&cf)).ok() else {
        return Ok(None);
    };
    let pclusters = compress::pcluster_blocks(&cf);
    let Some(pcluster_count) = usize::try_from(pclusters).ok() else {
        return Ok(None);
    };

    if pcluster_count >= totalidx {
        return Ok(None);
    }

    let ebase = align8(inode_header).saturating_add(Z_EROFS_MAP_HEADER_SIZE);
    let index_size = index_bytes(totalidx, ebase);
    let meta_total = ebase.saturating_add(index_size);

    let Some(inode) = inodes.get_mut(i) else {
        return Ok(None);
    };
    inode.nid = nid;
    inode.datalayout = EROFS_INODE_COMPRESSED_COMPACT;
    inode.data_blocks = pclusters;
    inode.compressed = Some(cf);

    Ok(Some(align_up(meta_total, SLOT_SIZE).unwrap_or(meta_total)))
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
    use std::io::Read;
    use std::path::Path;

    use super::{regular, special, symlink};
    use crate::Compression;
    use crate::compress::pcluster_blocks;
    use crate::dir::{EROFS_FT_DIR, EROFS_FT_REG_FILE, EROFS_FT_SYMLINK};
    use crate::inode::{
        COMPACT_INODE_SIZE, EROFS_INODE_COMPRESSED_COMPACT, EROFS_INODE_FLAT_INLINE,
        EROFS_INODE_FLAT_PLAIN,
    };
    use crate::layout::{InodeLayout, plan};
    use crate::source::{self, SizedFile};
    use crate::testutil::{compress_config, test_config};
    use crate::tree::TreeEntry;
    use crate::writer::image;

    fn open_reader(dir_path: &Path, ent: &TreeEntry) -> Box<dyn Read> {
        if ent.file_type == EROFS_FT_DIR || ent.file_type == EROFS_FT_SYMLINK || ent.size == 0 {
            return Box::new(std::io::empty());
        }
        let full = dir_path.join(ent.rel_path.strip_prefix('/').unwrap_or(&ent.rel_path));
        Box::new(std::fs::File::open(&full).expect("open"))
    }

    fn sized_files<'a>(
        entries: &[TreeEntry],
        readers: &'a mut [Box<dyn Read>],
    ) -> Vec<SizedFile<'a>> {
        entries
            .iter()
            .zip(readers.iter_mut())
            .map(|(entry, reader)| SizedFile {
                entry: entry.clone(),
                reader: reader.as_mut(),
            })
            .collect()
    }

    fn mkfs_from_dir(dir_path: &Path, config: &crate::MkfsConfig<'_>) -> Vec<u8> {
        let entries = source::collect_entries(dir_path).expect("collect_entries");
        let readers = |entries: &[TreeEntry]| {
            entries
                .iter()
                .map(|ent| open_reader(dir_path, ent))
                .collect::<Vec<_>>()
        };

        let mut pass1 = readers(&entries);
        let mut files1 = sized_files(&entries, &mut pass1);
        let planned = plan(&mut files1, config).expect("plan");

        let mut pass2 = readers(&entries);
        let mut files = sized_files(&entries, &mut pass2);

        let mut buf = Vec::new();
        image(&mut buf, &planned, &mut files, config).expect("image");
        buf
    }

    #[test]
    fn flat_inline_for_small_files() {
        // ARRANGE
        let entries = vec![
            TreeEntry {
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
            TreeEntry {
                rel_path: "/small".to_owned(),
                file_type: EROFS_FT_REG_FILE,
                size: 5,
                mode: 0o644,
                uid: 0,
                gid: 0,
                mtime: 0,
                mtime_nsec: 0,
                symlink_target: vec![],
                rdev: 0,
            },
        ];

        // ACT
        let mut readers: Vec<Box<dyn Read>> = vec![
            Box::new(std::io::empty()),
            Box::new(std::io::Cursor::new(b"hello".to_vec())),
        ];
        let planned = plan(
            &mut entries
                .into_iter()
                .zip(readers.iter_mut())
                .map(|(e, reader)| SizedFile {
                    entry: e,
                    reader: reader.as_mut(),
                })
                .collect::<Vec<_>>(),
            &test_config(1),
        )
        .expect("plan");
        let inodes = &planned.inodes;

        // ASSERT
        let file_inode = inodes
            .iter()
            .find(|inode| inode.rel_path == "/small")
            .expect("found");
        assert_eq!(file_inode.datalayout, EROFS_INODE_FLAT_INLINE);
        assert_eq!(file_inode.size, 5);
    }

    #[test]
    fn flat_plain_for_large_files() {
        // ARRANGE
        let entries = vec![
            TreeEntry {
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
            TreeEntry {
                rel_path: "/large".to_owned(),
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
        ];

        // ACT
        let mut readers: Vec<Box<dyn Read>> = vec![
            Box::new(std::io::empty()),
            Box::new(std::io::Cursor::new(vec![0_u8; 8192])),
        ];
        let planned = plan(
            &mut entries
                .into_iter()
                .zip(readers.iter_mut())
                .map(|(e, reader)| SizedFile {
                    entry: e,
                    reader: reader.as_mut(),
                })
                .collect::<Vec<_>>(),
            &test_config(1),
        )
        .expect("plan");
        let inodes = &planned.inodes;

        // ASSERT
        let file = inodes
            .iter()
            .find(|inode| inode.rel_path == "/large")
            .expect("found");
        assert_eq!(file.datalayout, EROFS_INODE_FLAT_PLAIN);
        assert_eq!(file.data_blocks, 2);
    }

    #[test]
    fn symlinks_always_inline() {
        // ARRANGE
        let dir = tempfile::tempdir().expect("tempdir");
        std::os::unix::fs::symlink("/target", dir.path().join("link")).expect("symlink");
        let entries = vec![
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
            },
            TreeEntry {
                rel_path: "/link".to_owned(),
                file_type: EROFS_FT_SYMLINK,
                size: 0,
                mode: 0o120_777,
                uid: 0,
                gid: 0,
                mtime: 1,
                mtime_nsec: 0,
                symlink_target: b"/target".to_vec(),
                rdev: 0,
            },
        ];
        let planned = plan(
            &mut entries
                .into_iter()
                .map(|e| SizedFile {
                    entry: e,
                    reader: Box::leak(Box::new(std::io::empty())),
                })
                .collect::<Vec<_>>(),
            &test_config(1),
        )
        .expect("plan");
        let inodes = &planned.inodes;
        let sym = inodes
            .iter()
            .find(|inode| inode.rel_path == "/link")
            .expect("found");

        // ACT & ASSERT
        assert_eq!(sym.datalayout, EROFS_INODE_FLAT_INLINE);
        assert_eq!(sym.file_type, EROFS_FT_SYMLINK);
    }

    #[test]
    fn layout_symlink_inline() {
        // ARRANGE
        let dir = tempfile::tempdir().expect("tempdir");
        std::os::unix::fs::symlink("/short", dir.path().join("link")).expect("symlink");
        let entries = vec![
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
            },
            TreeEntry {
                rel_path: "/link".to_owned(),
                file_type: EROFS_FT_SYMLINK,
                size: 0,
                mode: 0o120_777,
                uid: 0,
                gid: 0,
                mtime: 1,
                mtime_nsec: 0,
                symlink_target: b"/short".to_vec(),
                rdev: 0,
            },
        ];
        let planned = plan(
            &mut entries
                .into_iter()
                .map(|e| SizedFile {
                    entry: e,
                    reader: Box::leak(Box::new(std::io::empty())),
                })
                .collect::<Vec<_>>(),
            &test_config(1),
        )
        .expect("plan");
        let inodes = &planned.inodes;
        let link = inodes
            .iter()
            .find(|inode| inode.rel_path == "/link")
            .expect("found");

        // ACT & ASSERT
        assert_eq!(link.datalayout, EROFS_INODE_FLAT_INLINE);
        assert_eq!(link.data_blocks, 0);
    }

    #[test]
    fn layout_symlink_flat_plain() {
        // ARRANGE
        let long_target = "/".to_owned() + &"x".repeat(4080);
        let entries = vec![
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
            },
            TreeEntry {
                rel_path: "/longlink".to_owned(),
                file_type: EROFS_FT_SYMLINK,
                size: 0,
                mode: 0o120_777,
                uid: 0,
                gid: 0,
                mtime: 1,
                mtime_nsec: 0,
                symlink_target: long_target.as_bytes().to_vec(),
                rdev: 0,
            },
        ];
        let planned = plan(
            &mut entries
                .into_iter()
                .map(|e| SizedFile {
                    entry: e,
                    reader: Box::leak(Box::new(std::io::empty())),
                })
                .collect::<Vec<_>>(),
            &test_config(1),
        )
        .expect("plan");
        let inodes = &planned.inodes;
        let link = inodes
            .iter()
            .find(|inode| inode.rel_path == "/longlink")
            .expect("found");

        // ACT & ASSERT
        assert_eq!(link.datalayout, EROFS_INODE_FLAT_PLAIN);
        assert!(link.data_blocks > 0);
    }

    #[test]
    fn layout_regular_empty_file() {
        // ARRANGE
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(dir.path().join("empty"), b"").expect("write");
        let entries = vec![
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
            },
            TreeEntry {
                rel_path: "/empty".to_owned(),
                file_type: EROFS_FT_REG_FILE,
                size: 0,
                mode: 0o644,
                uid: 0,
                gid: 0,
                mtime: 1,
                mtime_nsec: 0,
                symlink_target: vec![],
                rdev: 0,
            },
        ];
        let planned = plan(
            &mut entries
                .into_iter()
                .map(|e| SizedFile {
                    entry: e,
                    reader: Box::leak(Box::new(std::io::empty())),
                })
                .collect::<Vec<_>>(),
            &test_config(1),
        )
        .expect("plan");
        let inodes = &planned.inodes;
        let empty = inodes
            .iter()
            .find(|inode| inode.rel_path == "/empty")
            .expect("found");

        // ACT & ASSERT
        assert_eq!(empty.datalayout, EROFS_INODE_FLAT_PLAIN);
        assert_eq!(empty.data_blocks, 0);
        assert_eq!(empty.size, 0);
    }

    #[test]
    fn layout_regular_entirely_inline() {
        // ARRANGE
        let entries = [
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
            },
            TreeEntry {
                rel_path: "/tiny".to_owned(),
                file_type: EROFS_FT_REG_FILE,
                size: 2,
                mode: 0o644,
                uid: 0,
                gid: 0,
                mtime: 1,
                mtime_nsec: 0,
                symlink_target: vec![],
                rdev: 0,
            },
        ];
        let planned = plan(
            &mut [
                SizedFile {
                    entry: entries.first().cloned().unwrap(),
                    reader: Box::leak(Box::new(std::io::empty())),
                },
                SizedFile {
                    entry: entries.get(1).cloned().unwrap(),
                    reader: &mut std::io::Cursor::new(b"hi".to_vec()),
                },
            ],
            &test_config(1),
        )
        .expect("plan");
        let inodes = &planned.inodes;
        let tiny = inodes
            .iter()
            .find(|inode| inode.rel_path == "/tiny")
            .expect("found");

        // ACT & ASSERT
        assert_eq!(tiny.datalayout, EROFS_INODE_FLAT_INLINE);
        assert_eq!(tiny.data_blocks, 0);
    }

    #[test]
    fn compressed_file_gets_compressed_full_layout() {
        // ARRANGE
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(dir.path().join("zeros"), vec![0_u8; 8192]).expect("write");
        let image = mkfs_from_dir(dir.path(), &compress_config(0));

        // ACT & ASSERT
        assert!(image.len() >= 4096);
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
        let entries = [
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
            },
            TreeEntry {
                rel_path: "/random".to_owned(),
                file_type: EROFS_FT_REG_FILE,
                size: 8192,
                mode: 0o644,
                uid: 0,
                gid: 0,
                mtime: 1,
                mtime_nsec: 0,
                symlink_target: vec![],
                rdev: 0,
            },
        ];
        let planned = plan(
            &mut [
                SizedFile {
                    entry: entries.first().cloned().unwrap(),
                    reader: Box::leak(Box::new(std::io::empty())),
                },
                SizedFile {
                    entry: entries.get(1).cloned().unwrap(),
                    reader: &mut std::io::Cursor::new(random_data),
                },
            ],
            &compress_config(0),
        )
        .expect("plan");
        let inodes = &planned.inodes;
        let file = inodes
            .iter()
            .find(|inode| inode.rel_path == "/random")
            .expect("found");

        // ACT & ASSERT
        assert_ne!(file.datalayout, EROFS_INODE_COMPRESSED_COMPACT);
        assert!(file.compressed.is_none());
    }

    #[test]
    fn compressed_empty_file_stays_flat() {
        // ARRANGE
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(dir.path().join("empty"), b"").expect("write");
        let entries = vec![
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
            },
            TreeEntry {
                rel_path: "/empty".to_owned(),
                file_type: EROFS_FT_REG_FILE,
                size: 0,
                mode: 0o644,
                uid: 0,
                gid: 0,
                mtime: 1,
                mtime_nsec: 0,
                symlink_target: vec![],
                rdev: 0,
            },
        ];
        let planned = plan(
            &mut entries
                .into_iter()
                .map(|e| SizedFile {
                    entry: e,
                    reader: Box::leak(Box::new(std::io::empty())),
                })
                .collect::<Vec<_>>(),
            &compress_config(0),
        )
        .expect("plan");
        let inodes = &planned.inodes;
        let file = inodes
            .iter()
            .find(|inode| inode.rel_path == "/empty")
            .expect("found");

        // ACT & ASSERT
        assert_eq!(file.datalayout, EROFS_INODE_FLAT_PLAIN);
        assert!(file.compressed.is_none());
    }

    #[test]
    fn compressed_small_file_stays_flat_when_no_block_savings() {
        // ARRANGE
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(dir.path().join("small"), vec![0_u8; 100]).expect("write");
        let entries = [
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
            },
            TreeEntry {
                rel_path: "/small".to_owned(),
                file_type: EROFS_FT_REG_FILE,
                size: 100,
                mode: 0o644,
                uid: 0,
                gid: 0,
                mtime: 1,
                mtime_nsec: 0,
                symlink_target: vec![],
                rdev: 0,
            },
        ];
        let planned = plan(
            &mut [
                SizedFile {
                    entry: entries.first().cloned().unwrap(),
                    reader: Box::leak(Box::new(std::io::empty())),
                },
                SizedFile {
                    entry: entries.get(1).cloned().unwrap(),
                    reader: &mut std::io::Cursor::new(vec![0_u8; 100]),
                },
            ],
            &compress_config(0),
        )
        .expect("plan");
        let inodes = &planned.inodes;
        let file = inodes
            .iter()
            .find(|inode| inode.rel_path == "/small")
            .expect("found");

        // ACT & ASSERT
        assert_eq!(file.datalayout, EROFS_INODE_FLAT_INLINE);
        assert!(file.compressed.is_none());
    }

    #[test]
    fn compressed_inode_data_blocks_is_pcluster_count() {
        // ARRANGE
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(dir.path().join("zeros"), vec![0_u8; 8192]).expect("write");
        let entries = [
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
            },
            TreeEntry {
                rel_path: "/zeros".to_owned(),
                file_type: EROFS_FT_REG_FILE,
                size: 8192,
                mode: 0o644,
                uid: 0,
                gid: 0,
                mtime: 1,
                mtime_nsec: 0,
                symlink_target: vec![],
                rdev: 0,
            },
        ];
        let planned = plan(
            &mut [
                SizedFile {
                    entry: entries.first().cloned().unwrap(),
                    reader: Box::leak(Box::new(std::io::empty())),
                },
                SizedFile {
                    entry: entries.get(1).cloned().unwrap(),
                    reader: &mut std::io::Cursor::new(vec![0_u8; 8192]),
                },
            ],
            &compress_config(0),
        )
        .expect("plan");
        let inodes = &planned.inodes;
        let file = inodes
            .iter()
            .find(|inode| inode.rel_path == "/zeros")
            .expect("found");
        let cf = file.compressed.as_ref().expect("compressed");
        let pclusters = pcluster_blocks(cf);

        // ACT & ASSERT
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
        let entries = vec![
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
            },
            TreeEntry {
                rel_path: "/compressible".to_owned(),
                file_type: EROFS_FT_REG_FILE,
                size: 8192,
                mode: 0o644,
                uid: 0,
                gid: 0,
                mtime: 1,
                mtime_nsec: 0,
                symlink_target: vec![],
                rdev: 0,
            },
            TreeEntry {
                rel_path: "/random".to_owned(),
                file_type: EROFS_FT_REG_FILE,
                size: 8192,
                mode: 0o644,
                uid: 0,
                gid: 0,
                mtime: 1,
                mtime_nsec: 0,
                symlink_target: vec![],
                rdev: 0,
            },
        ];
        let mut readers: Vec<Box<dyn Read>> = vec![
            Box::new(std::io::empty()),
            Box::new(std::io::Cursor::new(vec![0_u8; 8192])),
            Box::new(std::io::Cursor::new(random_data)),
        ];
        let planned = plan(
            &mut entries
                .into_iter()
                .zip(readers.iter_mut())
                .map(|(e, reader)| SizedFile {
                    entry: e,
                    reader: reader.as_mut(),
                })
                .collect::<Vec<_>>(),
            &compress_config(0),
        )
        .expect("plan");
        let inodes = &planned.inodes;
        let comp = inodes
            .iter()
            .find(|inode| inode.rel_path == "/compressible")
            .expect("found");
        let rand = inodes
            .iter()
            .find(|inode| inode.rel_path == "/random")
            .expect("found");

        // ACT & ASSERT
        assert_eq!(comp.datalayout, EROFS_INODE_COMPRESSED_COMPACT);
        assert!(comp.compressed.is_some());
        assert_ne!(rand.datalayout, EROFS_INODE_COMPRESSED_COMPACT);
        assert!(rand.compressed.is_none());
    }

    #[test]
    fn layout_functions_return_zero_for_missing_inode_index() {
        // ARRANGE
        let mut inodes = vec![InodeLayout {
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
            data_blkaddr: 0,
            data_blocks: 0,
            children: Vec::new(),
            symlink_target: Vec::new(),
            rdev: 0,
            compressed: None,
        }];
        let mut files = [SizedFile {
            entry: TreeEntry {
                rel_path: "/".to_owned(),
                file_type: EROFS_FT_REG_FILE,
                size: 0,
                mode: 0,
                uid: 0,
                gid: 0,
                mtime: 0,
                mtime_nsec: 0,
                symlink_target: vec![],
                rdev: 0,
            },
            reader: Box::leak(Box::new(std::io::empty())),
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
            &mut files,
        )
        .expect("regular for missing index");
        let special_advance = special(&mut inodes, 9, 1, COMPACT_INODE_SIZE);

        // ACT & ASSERT
        assert_eq!(symlink_advance, 0);
        assert_eq!(regular_advance, 0);
        assert_eq!(special_advance, 0);
    }
}
