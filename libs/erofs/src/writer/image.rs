//! Top-level EROFS image assembly and superblock emission.

extern crate alloc;

use alloc::collections::BTreeMap;
use std::io::Write;

use super::compressed;
use super::data;
use super::inode::write_header;
use super::util::{block_size_usize, slot_offset};
use crate::checked::{add, align_up, u32_from_usize};
use crate::error::{ErofsError, Result};
use crate::inode::COMPACT_INODE_SIZE;
use crate::layout::{self, ImagePlan, InodeLayout};
use crate::superblock::{self, SuperblockParams};

const ZERO_BLOCK: [u8; 4096] = [0_u8; 4096];

fn write_plain_blocks<W: Write>(
    writer: &mut W,
    inode: &InodeLayout,
    all_inodes: &[InodeLayout],
    path_to_idx: &BTreeMap<String, usize>,
    block_size: usize,
) -> Result<()> {
    let Some(data_bytes) = data::plain_blocks(inode, all_inodes, path_to_idx, block_size)? else {
        return Ok(());
    };
    writer.write_all(&data_bytes).map_err(ErofsError::Io)?;
    let full = usize::try_from(inode.data_blocks)
        .unwrap_or_default()
        .saturating_mul(block_size);
    let padding = full.saturating_sub(data_bytes.len());
    if padding == 0 {
        return Ok(());
    }
    let padding_slice = ZERO_BLOCK
        .get(..padding)
        .ok_or(ErofsError::Internal("padding exceeds ZERO_BLOCK"))?;

    writer.write_all(padding_slice).map_err(ErofsError::Io)
}

/// Build a complete EROFS image from a planned image plan into a `Write` sink.
pub fn write_image<W: Write>(
    writer: &mut W,
    plan: &ImagePlan,
    config: &crate::MkfsConfig<'_>,
) -> Result<()> {
    let block_size = block_size_usize();
    let inodes = &plan.inodes;
    let path_to_idx: BTreeMap<_, _> = inodes
        .iter()
        .enumerate()
        .map(|(index, inode)| (inode.rel_path.clone(), index))
        .collect();

    let meta_end = layout::compute_meta_end(inodes, plan.do_compress).max(block_size);
    let meta_end_aligned = align_up(meta_end, block_size).unwrap_or(meta_end);
    let mut meta_buf = vec![0_u8; meta_end];

    for inode in inodes {
        let slot_offset = slot_offset(inode.nid)?;
        let xattr_size = inode.xattr_payload.len();
        let inode_header_end = add(slot_offset, COMPACT_INODE_SIZE)
            .and_then(|offset| add(offset, xattr_size))
            .ok_or(ErofsError::Internal("inode header offset overflow"))?;

        write_header(&mut meta_buf, inode, slot_offset)?;

        if inode.compressed.is_some() {
            compressed::write_metadata(&mut meta_buf, inode, slot_offset)?;
        } else {
            data::write_inline_tail(
                &mut meta_buf,
                inode,
                inodes,
                &path_to_idx,
                inode_header_end,
                block_size,
            )?;
        }
    }

    let root_nid = inodes
        .first()
        .map_or(0, |inode| u16::try_from(inode.nid).ok().unwrap_or(u16::MAX));
    let blocks = plan
        .total_size
        .checked_div(block_size)
        .and_then(u32_from_usize)
        .unwrap_or(u32::MAX);

    superblock::write(
        &mut meta_buf,
        &SuperblockParams {
            root_nid,
            inos: u64::try_from(inodes.len()).ok().unwrap_or(u64::MAX),
            epoch: config.source_date_epoch,
            blocks,
            uuid: config.uuid,
            has_compression: plan.do_compress,
        },
    )?;
    superblock::write_checksum(&mut meta_buf)?;

    writer.write_all(&meta_buf).map_err(ErofsError::Io)?;

    let pad = meta_end_aligned.saturating_sub(meta_end);
    if pad > 0 {
        let padding_slice = ZERO_BLOCK
            .get(..pad)
            .ok_or(ErofsError::Internal("pad exceeds ZERO_BLOCK"))?;
        writer.write_all(padding_slice).map_err(ErofsError::Io)?;
    }

    for inode in inodes {
        if inode.compressed.is_some() {
            compressed::compressed_blocks(writer, inode)?;
        } else {
            write_plain_blocks(writer, inode, inodes, &path_to_idx, block_size)?;
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use std::io;

    use crate::MkfsConfig;
    use crate::SLOT_SIZE;
    use crate::dir::{EROFS_FT_DIR, EROFS_FT_REG_FILE};
    use crate::layout;
    use crate::source::SizedFile;
    use crate::superblock::{EROFS_SUPER_MAGIC_V1, EROFS_SUPER_OFFSET};
    use crate::testutil::{compress_config, test_config};
    use crate::tree::TreeEntry;

    fn run_write(planned: &layout::ImagePlan, cfg: &MkfsConfig<'_>) -> Vec<u8> {
        let mut image = Vec::new();
        super::write_image(&mut image, planned, cfg).expect("write_image");
        image
    }

    fn placeholder_data(e: &TreeEntry) -> Vec<u8> {
        if e.file_type == EROFS_FT_REG_FILE && e.size > 0 {
            vec![0_u8; usize::try_from(e.size).expect("size fits usize")]
        } else {
            Vec::new()
        }
    }

    fn plan_from_entries(entries: &[TreeEntry], cfg: &MkfsConfig<'_>) -> layout::ImagePlan {
        let mut datas: Vec<Vec<u8>> = entries.iter().map(placeholder_data).collect();
        let mut cursors: Vec<io::Cursor<&mut [u8]>> = datas
            .iter_mut()
            .map(|data| io::Cursor::new(data.as_mut_slice()))
            .collect();
        let mut files: Vec<SizedFile<'_>> = entries
            .iter()
            .zip(cursors.iter_mut())
            .map(|(entry, cursor)| SizedFile {
                entry: entry.clone(),
                reader: cursor,
            })
            .collect();
        layout::plan(&mut files, cfg).expect("plan")
    }

    #[test]
    fn write_image_empty_file_has_zero_startblk() {
        // ARRANGE
        let entries = &[
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
        let cfg = test_config(1);

        // ACT
        let planned = plan_from_entries(entries, &cfg);
        let image = run_write(&planned, &cfg);

        // ASSERT
        let empty = planned
            .inodes
            .iter()
            .find(|inode| inode.rel_path == "/empty")
            .expect("found");
        let slot_offset = usize::try_from(empty.nid).expect("nid fits usize") * SLOT_SIZE;
        let startblk = u32::from_le_bytes(
            image
                .get(slot_offset + 0x10..slot_offset + 0x14)
                .expect("start block bytes")
                .try_into()
                .expect("4 bytes"),
        );
        assert_eq!(startblk, 0);
    }

    #[test]
    fn superblock_at_correct_offset() {
        // ARRANGE
        let entries = &[TreeEntry {
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
        }];
        let cfg = test_config(1);

        // ACT
        let planned = plan_from_entries(entries, &cfg);
        let image = run_write(&planned, &cfg);

        // ASSERT
        let magic = u32::from_le_bytes(
            image
                .get(EROFS_SUPER_OFFSET..EROFS_SUPER_OFFSET + 4)
                .expect("magic bytes")
                .try_into()
                .expect("4 bytes"),
        );
        assert_eq!(magic, EROFS_SUPER_MAGIC_V1);
    }

    #[test]
    fn root_nid_matches_root_dir() {
        // ARRANGE
        let entries = &[TreeEntry {
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
        }];
        let cfg = test_config(1);

        // ACT
        let planned = plan_from_entries(entries, &cfg);
        let image = run_write(&planned, &cfg);

        // ASSERT
        let root_nid = u16::from_le_bytes(
            image
                .get(EROFS_SUPER_OFFSET + 0x0E..EROFS_SUPER_OFFSET + 0x10)
                .expect("root nid bytes")
                .try_into()
                .expect("2 bytes"),
        );
        let root = planned.inodes.first().expect("root inode");
        assert_eq!(
            root_nid,
            u16::try_from(root.nid).expect("root nid fits u16")
        );
    }

    #[test]
    fn root_nid_is_36_in_image() {
        // ARRANGE
        let entries = &[TreeEntry {
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
        }];
        let cfg = test_config(1);

        // ACT
        let planned = plan_from_entries(entries, &cfg);
        let image = run_write(&planned, &cfg);

        // ASSERT
        let root_nid = u16::from_le_bytes(
            image
                .get(EROFS_SUPER_OFFSET + 0x0E..EROFS_SUPER_OFFSET + 0x10)
                .expect("root nid bytes")
                .try_into()
                .expect("2 bytes"),
        );
        assert_eq!(root_nid, 36);
    }

    #[test]
    fn reproducible_output() {
        // ARRANGE
        let entries = &[
            TreeEntry {
                rel_path: "/".to_owned(),
                file_type: EROFS_FT_DIR,
                size: 0,
                mode: 0o40755,
                uid: 0,
                gid: 0,
                mtime: 1000,
                mtime_nsec: 0,
                symlink_target: vec![],
                rdev: 0,
            },
            TreeEntry {
                rel_path: "/a".to_owned(),
                file_type: EROFS_FT_REG_FILE,
                size: 3,
                mode: 0o644,
                uid: 0,
                gid: 0,
                mtime: 1000,
                mtime_nsec: 0,
                symlink_target: vec![],
                rdev: 0,
            },
            TreeEntry {
                rel_path: "/b".to_owned(),
                file_type: EROFS_FT_REG_FILE,
                size: 3,
                mode: 0o644,
                uid: 0,
                gid: 0,
                mtime: 1000,
                mtime_nsec: 0,
                symlink_target: vec![],
                rdev: 0,
            },
        ];
        let cfg = MkfsConfig {
            uuid: [1_u8; 16],
            ..test_config(1000)
        };

        // ACT
        let planned1 = plan_from_entries(entries, &cfg);
        let image1 = run_write(&planned1, &cfg);
        let planned2 = plan_from_entries(entries, &cfg);
        let image2 = run_write(&planned2, &cfg);

        // ASSERT
        assert_eq!(image1, image2);
    }

    #[test]
    fn write_image_with_selinux_xattr() {
        // ARRANGE
        let entries = &[
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
                rel_path: "/f".to_owned(),
                file_type: EROFS_FT_REG_FILE,
                size: 1,
                mode: 0o644,
                uid: 0,
                gid: 0,
                mtime: 0,
                mtime_nsec: 0,
                symlink_target: vec![],
                rdev: 0,
            },
        ];
        let fc =
            crate::FileContexts::from_reader("/.*    system_u:object_r:file_t:s0\n".as_bytes())
                .expect("fc");
        let cfg = MkfsConfig {
            file_contexts: Some(&fc),
            ..test_config(0)
        };

        // ACT
        let planned = plan_from_entries(entries, &cfg);
        let _: Vec<u8> = run_write(&planned, &cfg);

        // ASSERT
        let file = planned
            .inodes
            .iter()
            .find(|inode| inode.rel_path == "/f")
            .expect("found");
        assert!(!file.xattr_payload.is_empty());
    }

    #[test]
    fn write_compressed_image_valid_size() {
        // ARRANGE
        let entries = &[
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
        ];
        let cfg = compress_config(0);

        // ACT
        let planned = plan_from_entries(entries, &cfg);
        let image = run_write(&planned, &cfg);

        // ASSERT
        assert!(image.len().is_multiple_of(4096));
        assert!(image.len() >= 4096);
    }

    #[test]
    fn write_compressed_superblock_compr_cfgs() {
        // ARRANGE
        let entries = &[
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
                rel_path: "/zeros".to_owned(),
                file_type: EROFS_FT_REG_FILE,
                size: 4096,
                mode: 0o644,
                uid: 0,
                gid: 0,
                mtime: 0,
                mtime_nsec: 0,
                symlink_target: vec![],
                rdev: 0,
            },
        ];
        let cfg = compress_config(0);

        // ACT
        let planned = plan_from_entries(entries, &cfg);
        let image = run_write(&planned, &cfg);

        // ASSERT
        let cfg_off = EROFS_SUPER_OFFSET + 128;
        let cfg_size = u16::from_le_bytes(
            image
                .get(cfg_off..cfg_off + 2)
                .expect("compression config size bytes")
                .try_into()
                .expect("2b"),
        );
        assert_eq!(cfg_size, 6);
        assert_eq!(*image.get(cfg_off + 2).expect("format byte"), 0);
        assert_eq!(*image.get(cfg_off + 3).expect("windowlog byte"), 5);
    }
}
