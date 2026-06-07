//! Top-level EROFS image assembly and superblock emission.

extern crate alloc;

use alloc::collections::BTreeMap;
use alloc::string::String;
use std::io::{Read, Seek, SeekFrom, Write};

use super::compressed::write_file as write_compressed;
use super::data::{dir as write_dir, file as write_file, symlink as write_symlink};
use super::inode::write_header;
use super::util::{block_size_usize, slot_offset};
use crate::checked::{add, u32_from_usize, u64_from_usize};
use crate::dir::{EROFS_FT_DIR, EROFS_FT_REG_FILE, EROFS_FT_SYMLINK};
use crate::error::{ErofsError, Result};
use crate::inode::COMPACT_INODE_SIZE;
use crate::layout::ImagePlan;
use crate::superblock::{self, SuperblockParams};

/// Build a complete EROFS image from a planned image plan into a `Write + Seek + Read` sink.
pub fn write_image<W: Write + Seek + Read>(
    writer: &mut W,
    plan: &ImagePlan,
    config: &crate::MkfsConfig<'_>,
) -> Result<()> {
    let block_size = block_size_usize();
    let total_size = plan.total_size;
    let inodes = &plan.inodes;
    let has_compressed = plan.do_compress;

    writer
        .seek(SeekFrom::Start(
            u64_from_usize(total_size).saturating_sub(1),
        ))
        .map_err(ErofsError::Io)?;
    writer.write_all(&[0]).map_err(ErofsError::Io)?;
    writer.seek(SeekFrom::Start(0)).map_err(ErofsError::Io)?;

    let path_to_idx: BTreeMap<String, usize> = inodes
        .iter()
        .enumerate()
        .map(|(index, inode)| (inode.rel_path.clone(), index))
        .collect();

    for inode in inodes {
        let slot_offset = slot_offset(inode.nid)?;
        let xattr_size = inode.xattr_payload.len();
        let inode_header_end = add(slot_offset, COMPACT_INODE_SIZE)
            .and_then(|offset| add(offset, xattr_size))
            .ok_or(ErofsError::Internal("inode header offset overflow"))?;

        write_header(writer, inode, slot_offset)?;

        match inode.file_type {
            EROFS_FT_DIR => {
                write_dir(
                    writer,
                    inode,
                    inodes,
                    &path_to_idx,
                    inode_header_end,
                    block_size,
                )?;
            }
            EROFS_FT_SYMLINK => {
                write_symlink(writer, inode, inode_header_end, block_size)?;
            }
            EROFS_FT_REG_FILE if inode.compressed.is_some() => {
                write_compressed(writer, inode, slot_offset)?;
            }
            EROFS_FT_REG_FILE if inode.size > 0 => {
                write_file(writer, inode, inode_header_end, block_size)?;
            }
            _ => {}
        }
    }

    let root_nid = inodes
        .first()
        .map_or(0, |inode| u16::try_from(inode.nid).ok().unwrap_or(u16::MAX));
    let blocks = total_size
        .checked_div(block_size)
        .and_then(u32_from_usize)
        .unwrap_or(u32::MAX);

    superblock::write(
        writer,
        &SuperblockParams {
            root_nid,
            inos: u64::try_from(inodes.len()).ok().unwrap_or(u64::MAX),
            epoch: config.source_date_epoch,
            blocks,
            uuid: config.uuid,
            has_compression: has_compressed,
        },
    )?;
    superblock::write_checksum(writer)?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;

    use super::write_image;
    use crate::MkfsConfig;
    use crate::SLOT_SIZE;
    use crate::layout;
    use crate::layout::collect::FilesystemTreeSource;
    use crate::superblock::{EROFS_SUPER_MAGIC_V1, EROFS_SUPER_OFFSET};
    use crate::testutil::{compress_config, test_config};

    fn run_write(planned: &layout::ImagePlan, cfg: &MkfsConfig<'_>) -> Vec<u8> {
        let mut cursor = Cursor::new(Vec::new());
        write_image(&mut cursor, planned, cfg).expect("write_image");
        cursor.into_inner()
    }

    #[test]
    fn write_image_empty_file_has_zero_startblk() {
        // ARRANGE
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(dir.path().join("empty"), b"").expect("write");
        let cfg = test_config(1);

        // ACT
        let planned = layout::plan(&FilesystemTreeSource::new(dir.path()), &cfg).expect("plan");
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
        let dir = tempfile::tempdir().expect("tempdir");
        let cfg = test_config(1);

        // ACT
        let planned = layout::plan(&FilesystemTreeSource::new(dir.path()), &cfg).expect("plan");
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
        let dir = tempfile::tempdir().expect("tempdir");
        let cfg = test_config(1);

        // ACT
        let planned = layout::plan(&FilesystemTreeSource::new(dir.path()), &cfg).expect("plan");
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
        let dir = tempfile::tempdir().expect("tempdir");
        let cfg = test_config(1);

        // ACT
        let planned = layout::plan(&FilesystemTreeSource::new(dir.path()), &cfg).expect("plan");
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
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(dir.path().join("a"), b"aaa").expect("write");
        std::fs::write(dir.path().join("b"), b"bbb").expect("write");
        let cfg = MkfsConfig {
            uuid: [1_u8; 16],
            ..test_config(1000)
        };

        // ACT
        let planned1 = layout::plan(&FilesystemTreeSource::new(dir.path()), &cfg).expect("plan");
        let image1 = run_write(&planned1, &cfg);
        let planned2 = layout::plan(&FilesystemTreeSource::new(dir.path()), &cfg).expect("plan");
        let image2 = run_write(&planned2, &cfg);

        // ASSERT
        assert_eq!(image1, image2);
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
            file_contexts: Some(&fc),
            ..test_config(0)
        };

        // ACT
        let planned = layout::plan(&FilesystemTreeSource::new(dir.path()), &cfg).expect("plan");
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
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(dir.path().join("zeros"), vec![0_u8; 8192]).expect("write");
        let cfg = compress_config(0);

        // ACT
        let planned = layout::plan(&FilesystemTreeSource::new(dir.path()), &cfg).expect("plan");
        let image = run_write(&planned, &cfg);

        // ASSERT
        assert!(image.len().is_multiple_of(4096));
        assert!(image.len() >= 4096);
    }

    #[test]
    fn write_compressed_superblock_compr_cfgs() {
        // ARRANGE
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(dir.path().join("zeros"), vec![0_u8; 4096]).expect("write");
        let cfg = compress_config(0);

        // ACT
        let planned = layout::plan(&FilesystemTreeSource::new(dir.path()), &cfg).expect("plan");
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
