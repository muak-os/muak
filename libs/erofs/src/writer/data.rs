//! Plain and inline data serializers for files, directories, and symlinks.

use alloc::borrow::Cow;
use alloc::collections::BTreeMap;

use super::dir::{find_parent_nid, sorted_entries};
use super::sizes::{full_block_bytes, usize_from_u32};
use crate::checked::write_bytes;
use crate::dir;
use crate::dir::{EROFS_FT_DIR, EROFS_FT_REG_FILE, EROFS_FT_SYMLINK};
use crate::error::{ErofsError, Result};
use crate::inode::EROFS_INODE_FLAT_INLINE;
use crate::layout::InodeLayout;

fn plain_data<'a>(
    inode: &'a InodeLayout,
    all_inodes: &'a [InodeLayout],
    path_to_idx: &BTreeMap<String, usize>,
    block_size: usize,
) -> Result<Option<Cow<'a, [u8]>>> {
    match inode.file_type {
        EROFS_FT_DIR => {
            let parent_nid = find_parent_nid(inode, all_inodes, path_to_idx);
            let dir_entries = sorted_entries(inode, all_inodes, path_to_idx, parent_nid);
            let dir_data = dir::serialize_entries(&dir_entries);
            if inode.datalayout == EROFS_INODE_FLAT_INLINE {
                Ok(dir_data
                    .get(..full_block_bytes(inode.data_blocks, block_size)?)
                    .map(|data| Cow::Owned(data.to_vec())))
            } else {
                Ok(Some(Cow::Owned(dir_data)))
            }
        }
        EROFS_FT_SYMLINK => {
            if inode.datalayout == EROFS_INODE_FLAT_INLINE {
                Ok(inode
                    .symlink_target
                    .get(..full_block_bytes(inode.data_blocks, block_size)?)
                    .map(Cow::Borrowed))
            } else {
                Ok(Some(Cow::Borrowed(&inode.symlink_target)))
            }
        }
        EROFS_FT_REG_FILE if inode.compressed.is_none() && inode.size > 0 => {
            if inode.datalayout == EROFS_INODE_FLAT_INLINE {
                Ok(inode
                    .raw_data
                    .get(..full_block_bytes(inode.data_blocks, block_size)?)
                    .map(Cow::Borrowed))
            } else {
                Ok(Some(Cow::Borrowed(&inode.raw_data)))
            }
        }
        _ => Ok(None),
    }
}

pub(super) fn write_inline_tail(
    buf: &mut [u8],
    inode: &InodeLayout,
    all_inodes: &[InodeLayout],
    path_to_idx: &BTreeMap<String, usize>,
    inode_header_end: usize,
    block_size: usize,
) -> Result<()> {
    let full_block_data_len = full_block_bytes(inode.data_blocks, block_size)?;
    match inode.file_type {
        EROFS_FT_DIR => {
            let parent_nid = find_parent_nid(inode, all_inodes, path_to_idx);
            let dir_entries = sorted_entries(inode, all_inodes, path_to_idx, parent_nid);
            let dir_data = dir::serialize_entries(&dir_entries);
            let tail = dir_data
                .get(full_block_data_len..usize_from_u32(inode.size))
                .unwrap_or_default();
            if !tail.is_empty() && !write_bytes(buf, inode_header_end, tail) {
                return Err(ErofsError::Internal("inline tail write out of bounds"));
            }
            Ok(())
        }
        EROFS_FT_SYMLINK => {
            let tail = inode
                .symlink_target
                .get(full_block_data_len..inode.symlink_target.len())
                .unwrap_or_default();
            if !tail.is_empty() && !write_bytes(buf, inode_header_end, tail) {
                return Err(ErofsError::Internal("inline tail write out of bounds"));
            }
            Ok(())
        }
        EROFS_FT_REG_FILE if inode.size > 0 && inode.compressed.is_none() => {
            let tail = inode
                .raw_data
                .get(full_block_data_len..inode.raw_data.len())
                .unwrap_or_default();
            if !tail.is_empty() && !write_bytes(buf, inode_header_end, tail) {
                return Err(ErofsError::Internal("inline tail write out of bounds"));
            }
            Ok(())
        }
        _ => Ok(()),
    }
}

pub(super) fn plain_blocks<'a>(
    inode: &'a InodeLayout,
    all_inodes: &'a [InodeLayout],
    path_to_idx: &BTreeMap<String, usize>,
    block_size: usize,
) -> Result<Option<Cow<'a, [u8]>>> {
    plain_data(inode, all_inodes, path_to_idx, block_size)
}

#[cfg(test)]
mod tests {
    use std::io;

    use crate::dir::{EROFS_FT_DIR, EROFS_FT_REG_FILE, EROFS_FT_SYMLINK};
    use crate::inode::{EROFS_INODE_FLAT_INLINE, EROFS_INODE_FLAT_PLAIN};
    use crate::layout;
    use crate::source::SizedFile;
    use crate::testutil::test_config;
    use crate::tree::TreeEntry;
    use crate::writer::image;

    fn run_write(planned: &layout::ImagePlan, cfg: &crate::MkfsConfig<'_>) -> Vec<u8> {
        let mut buf = Vec::new();
        image(&mut buf, planned, cfg).expect("image");
        buf
    }

    fn placeholder_data(e: &TreeEntry) -> Vec<u8> {
        if e.file_type == EROFS_FT_REG_FILE && e.size > 0 {
            vec![0_u8; usize::try_from(e.size).expect("size fits usize")]
        } else {
            Vec::new()
        }
    }

    fn plan_from_entries(entries: &[TreeEntry], cfg: &crate::MkfsConfig<'_>) -> layout::ImagePlan {
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
    fn write_image_with_inline_file() {
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
        let cfg = test_config(1);

        // ACT
        let planned = plan_from_entries(entries, &cfg);
        let image = run_write(&planned, &cfg);

        // ASSERT
        let file_inode = planned
            .inodes
            .iter()
            .find(|inode| inode.rel_path == "/small")
            .expect("found");
        assert_eq!(file_inode.datalayout, EROFS_INODE_FLAT_INLINE);
        assert!(image.len() >= 4096);
    }

    #[test]
    fn write_dir_data_inline() {
        // ARRANGE
        let mut entry_entries = vec![TreeEntry {
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
        }];
        for index in 0..5_u8 {
            entry_entries.push(TreeEntry {
                rel_path: format!("/f{index}"),
                file_type: EROFS_FT_REG_FILE,
                size: 1,
                mode: 0o644,
                uid: 0,
                gid: 0,
                mtime: 0,
                mtime_nsec: 0,
                symlink_target: vec![],
                rdev: 0,
            });
        }
        let cfg = test_config(1);

        // ACT
        let planned = plan_from_entries(&entry_entries, &cfg);
        let image = run_write(&planned, &cfg);

        // ASSERT
        let root = planned.inodes.first().expect("root inode");
        assert_eq!(root.datalayout, EROFS_INODE_FLAT_INLINE);
        assert!(image.len() >= 4096);
    }

    #[test]
    fn write_dir_data_plain() {
        // ARRANGE
        let mut entry_entries = vec![TreeEntry {
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
        }];
        for index in 0_u16..339 {
            entry_entries.push(TreeEntry {
                rel_path: format!("/file_{index:03}.txt"),
                file_type: EROFS_FT_REG_FILE,
                size: 1,
                mode: 0o644,
                uid: 0,
                gid: 0,
                mtime: 0,
                mtime_nsec: 0,
                symlink_target: vec![],
                rdev: 0,
            });
        }
        let cfg = test_config(1);

        // ACT
        let planned = plan_from_entries(&entry_entries, &cfg);

        // ASSERT
        let root = planned.inodes.first().expect("root inode");
        assert_eq!(root.datalayout, EROFS_INODE_FLAT_PLAIN);
        assert!(root.data_blocks > 0);
    }

    #[test]
    fn write_symlink_data_inline() {
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
                rel_path: "/link".to_owned(),
                file_type: EROFS_FT_SYMLINK,
                size: 0,
                mode: 0o120_777,
                uid: 0,
                gid: 0,
                mtime: 0,
                mtime_nsec: 0,
                symlink_target: b"/short".to_vec(),
                rdev: 0,
            },
        ];
        let cfg = test_config(1);

        // ACT
        let planned = plan_from_entries(entries, &cfg);
        let image = run_write(&planned, &cfg);

        // ASSERT
        let link = planned
            .inodes
            .iter()
            .find(|inode| inode.rel_path == "/link")
            .expect("found");
        assert_eq!(link.datalayout, EROFS_INODE_FLAT_INLINE);
        assert!(image.len() >= 4096);
    }

    #[test]
    fn write_symlink_data_plain() {
        // ARRANGE
        let long_target = "/".to_owned() + &"x".repeat(4080);
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
                rel_path: "/longlink".to_owned(),
                file_type: EROFS_FT_SYMLINK,
                size: 0,
                mode: 0o120_777,
                uid: 0,
                gid: 0,
                mtime: 0,
                mtime_nsec: 0,
                symlink_target: long_target.as_bytes().to_vec(),
                rdev: 0,
            },
        ];
        let cfg = test_config(1);

        // ACT
        let planned = plan_from_entries(entries, &cfg);
        let _: Vec<u8> = run_write(&planned, &cfg);

        // ASSERT
        let link = planned
            .inodes
            .iter()
            .find(|inode| inode.rel_path == "/longlink")
            .expect("found");
        assert_eq!(link.datalayout, EROFS_INODE_FLAT_PLAIN);
        assert!(link.data_blocks > 0);
    }

    #[test]
    fn write_file_data_with_inline_tail() {
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
                rel_path: "/partial".to_owned(),
                file_type: EROFS_FT_REG_FILE,
                size: 4100,
                mode: 0o644,
                uid: 0,
                gid: 0,
                mtime: 0,
                mtime_nsec: 0,
                symlink_target: vec![],
                rdev: 0,
            },
        ];
        let cfg = test_config(1);

        // ACT
        let planned = plan_from_entries(entries, &cfg);
        let _: Vec<u8> = run_write(&planned, &cfg);

        // ASSERT
        let file = planned
            .inodes
            .iter()
            .find(|inode| inode.rel_path == "/partial")
            .expect("found");
        assert_eq!(file.datalayout, EROFS_INODE_FLAT_INLINE);
        assert!(file.data_blocks > 0);
    }

    #[test]
    fn write_inline_data_only_tail() {
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
                rel_path: "/tiny".to_owned(),
                file_type: EROFS_FT_REG_FILE,
                size: 2,
                mode: 0o644,
                uid: 0,
                gid: 0,
                mtime: 0,
                mtime_nsec: 0,
                symlink_target: vec![],
                rdev: 0,
            },
        ];
        let cfg = test_config(1);

        // ACT
        let planned = plan_from_entries(entries, &cfg);
        let _: Vec<u8> = run_write(&planned, &cfg);

        // ASSERT
        let file = planned
            .inodes
            .iter()
            .find(|inode| inode.rel_path == "/tiny")
            .expect("found");
        assert_eq!(file.data_blocks, 0);
    }

    #[test]
    fn write_file_data_plain_layout() {
        // ARRANGE
        let data = vec![0xAB_u8; 4096];
        let mut data_copy = data.clone();
        let mut data_cursor = io::Cursor::new(data_copy.as_mut_slice());
        let files = &mut [
            SizedFile {
                entry: TreeEntry {
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
                reader: &mut io::empty(),
            },
            SizedFile {
                entry: TreeEntry {
                    rel_path: "/full".to_owned(),
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
                reader: &mut data_cursor,
            },
        ];
        let cfg = test_config(1);

        // ACT
        let planned = layout::plan(files, &cfg).expect("plan");
        let image = run_write(&planned, &cfg);

        // ASSERT
        let file = planned
            .inodes
            .iter()
            .find(|inode| inode.rel_path == "/full")
            .expect("found");
        assert_eq!(file.datalayout, EROFS_INODE_FLAT_PLAIN);
        let data_start = usize::try_from(file.data_blkaddr).expect("blkaddr fits usize") * 4096;
        assert_eq!(
            image
                .get(data_start..data_start + 4096)
                .expect("plain data bytes"),
            data.as_slice()
        );
    }
}
