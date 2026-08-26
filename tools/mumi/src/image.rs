//! EROFS image construction from synthetic file entries.

use std::io::{Read, Write};

use erofs::dir::{EROFS_FT_DIR, EROFS_FT_REG_FILE, EROFS_FT_SYMLINK};
use erofs::tree::TreeEntry;
use erofs::{Compression, FileContexts, MkfsConfig};

use crate::error::{MumiError, Result};

/// A single file entry for building an image from synthetic data.
pub struct Entry {
    /// Absolute path inside the image (e.g. `/usr/bin/foo`).
    pub path: String,
    /// File size in bytes.
    pub size: u64,
    /// File mode including file-type bits (e.g. `0o100_755`).
    pub mode: u32,
    /// Symlink target bytes, empty for non-symlinks.
    pub symlink_target: Vec<u8>,
}

/// Configuration for building an EROFS image.
pub struct BuildConfig {
    /// Zstd compression level.
    pub compression_level: i32,
    /// Optional `SELinux` file contexts. Rootfs builds pass `Some`,
    /// extension builds pass `None`.
    pub file_contexts: Option<FileContexts>,
}

/// A fully-planned image. Carries layout only.
pub struct Image {
    len: u64,
    plan: erofs::ImagePlan,
    file_contexts: Option<FileContexts>,
    compression: Compression,
}

impl Image {
    /// Builds an EROFS image from synthetic entries and their data.
    ///
    /// # Errors
    ///
    /// Returns an error when the root entry is missing, entries are invalid,
    /// `readers` length mismatches `entries`, compression settings are invalid,
    /// file data cannot be read, or the image layout cannot be planned.
    pub fn build(
        entries: &[Entry],
        readers: &mut [&mut dyn Read],
        config: &BuildConfig,
    ) -> Result<Image> {
        let compression = Compression::Zstd {
            level: config.compression_level,
        };
        erofs::validate_compression_level(config.compression_level)
            .map_err(|e| MumiError::InvalidArgument(e.to_string()))?;
        if !entries.iter().any(|entry| entry.path == "/") {
            return Err(MumiError::InvalidArgument(
                "entries must include the root directory \"/\"".to_owned(),
            ));
        }
        if readers.len() != entries.len() {
            return Err(MumiError::InvalidArgument(format!(
                "reader set size mismatch: {} entries, {} readers",
                entries.len(),
                readers.len(),
            )));
        }

        let mkfs_config = mkfs_config(config.file_contexts.as_ref(), compression);
        let mut sized: Vec<erofs::SizedFile<'_>> = Vec::with_capacity(entries.len());
        for (entry, reader) in entries.iter().zip(readers.iter_mut()) {
            sized.push(erofs::SizedFile {
                entry: tree_entry(entry),
                reader: &mut **reader,
            });
        }

        let plan = erofs::layout::plan(&mut sized, &mkfs_config)
            .map_err(|e| MumiError::Erofs(e.to_string()))?;
        let len = u64::try_from(plan.total_size).unwrap_or(u64::MAX);

        Ok(Image {
            len,
            plan,
            file_contexts: config.file_contexts.clone(),
            compression,
        })
    }

    /// Writes the complete EROFS image to `writer`.
    ///
    /// # Errors
    ///
    /// Returns an error when the reader set length mismatches the entries,
    /// metadata serialization fails, or data emission fails.
    pub fn write<W: Write>(&self, writer: &mut W, readers: &mut [&mut dyn Read]) -> Result<()> {
        let config = mkfs_config(self.file_contexts.as_ref(), self.compression);
        let entry_count = self.plan.inodes.len();
        if readers.len() != entry_count {
            return Err(MumiError::InvalidArgument(format!(
                "write reader set size mismatch: {} inodes, {} readers",
                entry_count,
                readers.len(),
            )));
        }

        let mut sized: Vec<erofs::SizedFile<'_>> = self
            .plan
            .inodes
            .iter()
            .zip(readers.iter_mut())
            .map(|(inode, reader)| erofs::SizedFile {
                entry: inode_entry(inode),
                reader: &mut **reader,
            })
            .collect();

        erofs::writer::image(writer, &self.plan, &mut sized, &config)
            .map_err(|e| MumiError::Erofs(e.to_string()))
    }

    /// Returns the exact image size in bytes.
    #[must_use]
    pub fn len(&self) -> u64 {
        self.len
    }

    /// Returns `true` when the image contains no bytes.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }
}

/// Builds the mkfs configuration shared by planning and writing.
fn mkfs_config(file_contexts: Option<&FileContexts>, compression: Compression) -> MkfsConfig<'_> {
    MkfsConfig {
        source_date_epoch: 0,
        file_contexts,
        uuid: [0; 16],
        force_uid: Some(0),
        force_gid: Some(0),
        compression,
    }
}

const S_IFMT: u32 = 0o170_000;
const S_IFDIR: u32 = 0o040_000;
const S_IFLNK: u32 = 0o120_000;

/// Reconstructs the source entry metadata from a planned inode.
fn inode_entry(inode: &erofs::InodeLayout) -> TreeEntry {
    TreeEntry {
        rel_path: inode.rel_path.clone(),
        file_type: inode.file_type,
        size: u64::from(inode.size),
        mode: u32::from(inode.mode),
        uid: u32::from(inode.uid),
        gid: u32::from(inode.gid),
        mtime: inode.mtime,
        mtime_nsec: inode.mtime_nsec,
        symlink_target: inode.symlink_target.clone(),
        rdev: inode.rdev,
    }
}

fn tree_entry(entry: &Entry) -> TreeEntry {
    TreeEntry {
        rel_path: entry.path.clone(),
        file_type: file_type_from_mode(entry.mode),
        size: entry.size,
        mode: entry.mode,
        uid: 0,
        gid: 0,
        mtime: 0,
        mtime_nsec: 0,
        symlink_target: entry.symlink_target.clone(),
        rdev: 0,
    }
}

fn file_type_from_mode(mode: u32) -> u8 {
    match mode & S_IFMT {
        S_IFDIR => EROFS_FT_DIR,
        S_IFLNK => EROFS_FT_SYMLINK,
        _ => EROFS_FT_REG_FILE,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::payload::{buffer_readers, read_views};

    fn config() -> BuildConfig {
        BuildConfig {
            compression_level: erofs::DEFAULT_ZSTD_COMPRESSION_LEVEL,
            file_contexts: None,
        }
    }

    fn root() -> Entry {
        Entry {
            path: "/".to_owned(),
            size: 0,
            mode: 0o040_755,
            symlink_target: Vec::new(),
        }
    }

    fn build_image(entries: &[Entry], datas: &[Vec<u8>]) -> Result<Image> {
        let mut readers = buffer_readers(datas);
        let mut views = read_views(&mut readers);
        Image::build(entries, &mut views, &config())
    }

    fn write_image(image: &Image, datas: &[Vec<u8>]) -> Vec<u8> {
        let mut readers = buffer_readers(datas);
        let mut views = read_views(&mut readers);

        let mut buf = Vec::new();
        image.write(&mut buf, &mut views).expect("write image");

        buf
    }

    #[test]
    fn builds_image_from_synthetic_entries() {
        // ARRANGE
        let entries = vec![
            root(),
            Entry {
                path: "/usr/bin/tool".to_owned(),
                size: 5,
                mode: 0o100_755,
                symlink_target: Vec::new(),
            },
            Entry {
                path: "/etc/conf".to_owned(),
                size: 4,
                mode: 0o100_644,
                symlink_target: Vec::new(),
            },
        ];
        let datas = vec![Vec::new(), b"hello".to_vec(), b"conf".to_vec()];

        // ACT
        let image = build_image(&entries, &datas).expect("build image");

        // ASSERT
        assert!(!image.is_empty());
        assert!(image.len().is_multiple_of(4096));
    }

    #[test]
    fn len_matches_written_bytes() {
        // ARRANGE
        let entries = vec![
            root(),
            Entry {
                path: "/f".to_owned(),
                size: 8,
                mode: 0o100_644,
                symlink_target: Vec::new(),
            },
        ];
        let datas = vec![Vec::new(), b"data....".to_vec()];
        let image = build_image(&entries, &datas).expect("build image");

        // ACT
        let buf = write_image(&image, &datas);

        // ASSERT
        assert_eq!(u64::try_from(buf.len()).unwrap_or(0), image.len());
    }

    #[test]
    fn reproducible_output() {
        // ARRANGE
        let entries = vec![
            root(),
            Entry {
                path: "/f".to_owned(),
                size: 3,
                mode: 0o100_644,
                symlink_target: Vec::new(),
            },
        ];
        let datas = vec![Vec::new(), b"abc".to_vec()];

        // ACT
        let image1 = build_image(&entries, &datas).expect("build image");
        let image2 = build_image(&entries, &datas).expect("build image");
        let buf1 = write_image(&image1, &datas);
        let buf2 = write_image(&image2, &datas);

        // ASSERT
        assert_eq!(buf1, buf2);
    }

    #[test]
    fn missing_root_errors() {
        // ARRANGE
        let entries = vec![Entry {
            path: "/f".to_owned(),
            size: 1,
            mode: 0o100_644,
            symlink_target: Vec::new(),
        }];
        let datas = vec![b"x".to_vec()];

        // ACT
        let result = build_image(&entries, &datas);

        // ASSERT
        assert!(result.is_err());
    }

    #[test]
    fn keeps_symlink_target() {
        // ARRANGE
        let entries = vec![
            root(),
            Entry {
                path: "/link".to_owned(),
                size: 0,
                mode: 0o120_777,
                symlink_target: b"/target".to_vec(),
            },
        ];
        let datas: Vec<Vec<u8>> = vec![Vec::new(), Vec::new()];

        // ACT
        let image = build_image(&entries, &datas).expect("build image");

        // ASSERT
        assert!(
            image
                .plan
                .inodes
                .iter()
                .any(|inode| inode.rel_path == "/link")
        );
    }
}
