//! EROFS image writer producing raw image bytes.

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use crate::dir::{self, DirEntry, EROFS_FT_DIR, EROFS_FT_REG_FILE, EROFS_FT_SYMLINK};
use crate::error::Result;
use crate::inode::{self, COMPACT_INODE_SIZE, CompactInodeParams, EROFS_INODE_FLAT_INLINE};
use crate::layout::{self, InodeLayout};
use crate::superblock::{self, SuperblockParams};
use crate::{BLOCK_SIZE, SLOT_SIZE};

/// Build a complete EROFS image from the planned layout.
pub fn write_image(inodes: &[InodeLayout], epoch: u64, uuid: [u8; 16]) -> Result<Vec<u8>> {
    let bs = BLOCK_SIZE as usize;
    let total_size = layout::total_image_size(inodes);
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
            EROFS_FT_REG_FILE if inode.size > 0 => {
                write_file_data(&mut image, inode, inode_header_end, bs)?;
            }
            _ => {}
        }
    }

    let root_nid = inodes.first().map_or(0, |i| i.nid as u16);
    let blocks = (total_size / bs) as u32;

    superblock::write_superblock(
        &mut image,
        &SuperblockParams {
            root_nid,
            inos: inodes.len() as u64,
            epoch,
            blocks,
            uuid,
        },
    );
    superblock::write_checksum(&mut image);

    Ok(image)
}

/// Write the 32-byte compact inode header and xattr payload into the image.
fn write_inode_header(image: &mut [u8], inode: &InodeLayout, slot_offset: usize) {
    let startblk = if inode.data_blocks > 0 {
        inode.data_blkaddr
    } else {
        u32::MAX
    };

    let i_u = if inode.file_type != EROFS_FT_DIR
        && inode.file_type != EROFS_FT_REG_FILE
        && inode.file_type != EROFS_FT_SYMLINK
    {
        inode.rdev
    } else {
        startblk
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
fn write_plain_data(image: &mut [u8], data: &[u8], data_blkaddr: u32, data_blocks: u32, bs: usize) {
    let data_start = data_blkaddr as usize * bs;
    let data_len = data_blocks as usize * bs;
    image[data_start..data_start + data_len].copy_from_slice(&data[..data_len]);
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
        write_plain_data(image, &dir_data, inode.data_blkaddr, inode.data_blocks, bs);
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
    use crate::inode::{EROFS_INODE_FLAT_INLINE, EROFS_INODE_FLAT_PLAIN};
    use crate::superblock::{EROFS_SUPER_MAGIC_V1, EROFS_SUPER_OFFSET};

    fn test_config(epoch: u64) -> MkfsConfig<'static> {
        MkfsConfig {
            source_date_epoch: epoch,
            file_contexts: None,
            uuid: [0; 16],
            force_uid: None,
            force_gid: None,
        }
    }

    #[test]
    fn write_image_empty_file_has_max_startblk() {
        // ARRANGE
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(dir.path().join("empty"), b"").expect("write");
        let cfg = test_config(1);

        // ACT
        let inodes = layout::plan(dir.path(), &cfg).expect("plan");
        let image = write_image(&inodes, 1, [0; 16]).expect("write");

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
        assert_eq!(startblk, u32::MAX);
    }

    #[test]
    fn find_parent_nid_for_root() {
        // ARRANGE
        let dir = tempfile::tempdir().expect("tempdir");
        let cfg = test_config(1);

        // ACT
        let inodes = layout::plan(dir.path(), &cfg).expect("plan");
        let _image = write_image(&inodes, 1, [0; 16]).expect("write");

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
        let image = write_image(&inodes, 1, [0; 16]).expect("write");

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
        let image = write_image(&inodes, 1, [0; 16]).expect("write");

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
        let image = write_image(&inodes, 1, [0; 16]).expect("write");

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
        };

        // ACT
        let inodes1 = layout::plan(dir.path(), &cfg).expect("plan");
        let image1 = write_image(&inodes1, 1000, uuid).expect("write");
        let inodes2 = layout::plan(dir.path(), &cfg).expect("plan");
        let image2 = write_image(&inodes2, 1000, uuid).expect("write");

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
        let image = write_image(&inodes, 0, [0; 16]).expect("write");

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
        let image = write_image(&inodes, 1, [0; 16]).expect("write");

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
        let image = write_image(&inodes, 1, [0; 16]).expect("write");

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
        let image = write_image(&inodes, 1, [0; 16]).expect("write");

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
        let _image = write_image(&inodes, 1, [0; 16]).expect("write");

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
        let _image = write_image(&inodes, 1, [0; 16]).expect("write");

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
        let _image = write_image(&inodes, 1, [0; 16]).expect("write");

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
        let _image = write_image(&inodes, 1, [0; 16]).expect("write");

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
        let image = write_image(&inodes, 1, [0; 16]).expect("write");

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
        };

        // ACT
        let inodes = layout::plan(dir.path(), &cfg).expect("plan");
        let _image = write_image(&inodes, 0, [0; 16]).expect("write");

        // ASSERT
        let file = inodes.iter().find(|i| i.rel_path == "/f").expect("found");
        assert!(!file.xattr_payload.is_empty());
    }
}
