//! EROFS image construction from synthetic file entries.

use core::cell::RefCell;
use std::io::{self, Read, Write};

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

/// TODO: Delete this.
/// Positional data source for image file contents.
///
/// `read(index, …)` returns bytes for the file at positional `index` into the
/// `entries` slice passed to [`build`]. Mumi wraps this in per-file adapters
/// that erofs sees as `Read`.
pub trait Reader {
    /// Reads the next chunk of bytes for the file at `index`.
    ///
    /// Returns the number of bytes copied into `buf`, or `0` when the file is
    /// exhausted or `index` is out of range.
    ///
    /// # Errors
    ///
    /// Returns an error when the underlying data source cannot be read.
    fn read(&mut self, index: usize, buf: &mut [u8]) -> io::Result<usize>;
}

/// A fully-planned image. Owns all data internally; self-contained and
/// `Send` once built.
pub struct Image {
    name: String,
    len: u64,
    plan: erofs::ImagePlan,
    file_contexts: Option<FileContexts>,
    compression: Compression,
}

impl Image {
    /// Returns the logical identity passed to [`build`].
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
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

    /// Writes the complete EROFS image to `writer`.
    ///
    /// # Errors
    ///
    /// Returns an error when metadata serialization or data emission fails.
    pub fn write<W: Write>(&self, writer: &mut W) -> Result<()> {
        let config = MkfsConfig {
            source_date_epoch: 0,
            file_contexts: self.file_contexts.as_ref(),
            uuid: [0; 16],
            force_uid: Some(0),
            force_gid: Some(0),
            compression: self.compression,
        };
        erofs::writer::image(writer, &self.plan, &config)
            .map_err(|e| MumiError::Erofs(e.to_string()))
    }
}

/// Builds an EROFS image named `name` from synthetic entries and their data.
///
/// The root entry is added automatically when `entries` does not already
/// contain a path of `/`. File data is read eagerly during planning, so the
/// returned [`Image`] is self-contained and no longer borrows `readers`.
///
/// # Errors
///
/// Returns an error when entries are invalid, compression settings are invalid,
/// file data cannot be read, or the image layout cannot be planned.
pub fn build(
    name: &str,
    entries: &[Entry],
    readers: &mut dyn Reader,
    config: &BuildConfig,
) -> Result<Image> {
    let compression = Compression::Zstd {
        level: config.compression_level,
    };
    erofs::validate_compression_level(config.compression_level)
        .map_err(|e| MumiError::InvalidArgument(e.to_string()))?;
    let cell = RefCell::new(readers);
    let mut adapters: Vec<FileAdapter<'_, '_>> = Vec::with_capacity(entries.len());
    for (index, _) in entries.iter().enumerate() {
        adapters.push(FileAdapter {
            reader: &cell,
            index,
        });
    }

    let mkfs_config = MkfsConfig {
        source_date_epoch: 0,
        file_contexts: config.file_contexts.as_ref(),
        uuid: [0; 16],
        force_uid: Some(0),
        force_gid: Some(0),
        compression,
    };

    let has_root = entries.iter().any(|entry| entry.path == "/");
    let mut sized: Vec<erofs::SizedFile<'_>> = Vec::with_capacity(entries.len().saturating_add(1));
    let mut empty = io::empty();
    if !has_root {
        sized.push(erofs::SizedFile {
            entry: root_entry(),
            reader: &mut empty,
        });
    }
    for (entry, adapter) in entries.iter().zip(adapters.iter_mut()) {
        sized.push(erofs::SizedFile {
            entry: tree_entry(entry),
            reader: adapter,
        });
    }

    let plan = erofs::layout::plan(&mut sized, &mkfs_config)
        .map_err(|e| MumiError::Erofs(e.to_string()))?;
    let len = u64::try_from(plan.total_size).unwrap_or(u64::MAX);

    Ok(Image {
        name: name.to_owned(),
        len,
        plan,
        file_contexts: config.file_contexts.clone(),
        compression,
    })
}

/// A per-file `Read` adapter that forwards to the positional [`Reader`].
struct FileAdapter<'a, 'r> {
    reader: &'a RefCell<&'r mut dyn Reader>,
    index: usize,
}

impl Read for FileAdapter<'_, '_> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        self.reader.borrow_mut().read(self.index, buf)
    }
}

const S_IFMT: u32 = 0o170_000;
const S_IFDIR: u32 = 0o040_000;
const S_IFLNK: u32 = 0o120_000;

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

fn root_entry() -> TreeEntry {
    TreeEntry {
        rel_path: "/".to_owned(),
        file_type: EROFS_FT_DIR,
        size: 0,
        mode: 0o40755,
        uid: 0,
        gid: 0,
        mtime: 0,
        mtime_nsec: 0,
        symlink_target: Vec::new(),
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

    struct TestReader {
        files: Vec<Vec<u8>>,
        positions: Vec<usize>,
    }

    impl TestReader {
        fn new(files: Vec<Vec<u8>>) -> Self {
            Self {
                positions: vec![0; files.len()],
                files,
            }
        }
    }

    impl Reader for TestReader {
        fn read(&mut self, index: usize, buf: &mut [u8]) -> io::Result<usize> {
            let file = self
                .files
                .get(index)
                .ok_or_else(|| io::Error::other("file out of bounds"))?;
            let position = self
                .positions
                .get_mut(index)
                .ok_or_else(|| io::Error::other("position out of bounds"))?;
            let remaining = file.len().saturating_sub(*position);
            let n = remaining.min(buf.len());
            let data = file
                .get(*position..position.saturating_add(n))
                .unwrap_or_default();
            buf.get_mut(..n)
                .ok_or_else(|| io::Error::other("buffer too small"))?
                .copy_from_slice(data);
            *position = position.saturating_add(n);

            Ok(n)
        }
    }

    fn config() -> BuildConfig {
        BuildConfig {
            compression_level: erofs::DEFAULT_ZSTD_COMPRESSION_LEVEL,
            file_contexts: None,
        }
    }

    #[test]
    fn builds_image_from_synthetic_entries() {
        // ARRANGE
        let entries = vec![
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
        let mut reader = TestReader::new(vec![b"hello".to_vec(), b"conf".to_vec()]);

        // ACT
        let image = build("test", &entries, &mut reader, &config()).expect("build image");

        // ASSERT
        assert_eq!(image.name(), "test");
        assert!(!image.is_empty());
        assert!(image.len().is_multiple_of(4096));
    }

    #[test]
    fn len_matches_written_bytes() {
        // ARRANGE
        let entries = vec![Entry {
            path: "/f".to_owned(),
            size: 8,
            mode: 0o100_644,
            symlink_target: Vec::new(),
        }];
        let mut reader = TestReader::new(vec![b"data....".to_vec()]);
        let image = build("rootfs", &entries, &mut reader, &config()).expect("build image");

        // ACT
        let mut buf = Vec::new();
        image.write(&mut buf).expect("write image");

        // ASSERT
        assert_eq!(u64::try_from(buf.len()).unwrap_or(0), image.len());
    }

    #[test]
    fn reproducible_output() {
        // ARRANGE
        let entries = vec![Entry {
            path: "/f".to_owned(),
            size: 3,
            mode: 0o100_644,
            symlink_target: Vec::new(),
        }];

        // ACT
        let mut reader = TestReader::new(vec![b"abc".to_vec()]);
        let image1 = build("rootfs", &entries, &mut reader, &config()).expect("build 1");
        let mut reader = TestReader::new(vec![b"abc".to_vec()]);
        let image2 = build("rootfs", &entries, &mut reader, &config()).expect("build 2");
        let mut buf1 = Vec::new();
        image1.write(&mut buf1).expect("write 1");
        let mut buf2 = Vec::new();
        image2.write(&mut buf2).expect("write 2");

        // ASSERT
        assert_eq!(buf1, buf2);
    }

    #[test]
    fn name_returns_identity() {
        // ARRANGE
        let mut reader = TestReader::new(Vec::new());

        // ACT
        let image = build("muak-os/qemu", &[], &mut reader, &config()).expect("build image");

        // ASSERT
        assert_eq!(image.name(), "muak-os/qemu");
    }

    #[test]
    fn prepends_root_when_absent() {
        // ARRANGE
        let entries = vec![Entry {
            path: "/f".to_owned(),
            size: 1,
            mode: 0o100_644,
            symlink_target: Vec::new(),
        }];
        let mut reader = TestReader::new(vec![b"x".to_vec()]);

        // ACT
        let image = build("rootfs", &entries, &mut reader, &config()).expect("build image");
        let mut buf = Vec::new();
        image.write(&mut buf).expect("write image");

        // ASSERT
        assert!(!buf.is_empty());
    }

    #[test]
    fn keeps_symlink_target() {
        // ARRANGE
        let entries = vec![Entry {
            path: "/link".to_owned(),
            size: 0,
            mode: 0o120_777,
            symlink_target: b"/target".to_vec(),
        }];
        let mut reader = TestReader::new(Vec::new());

        // ACT
        let image = build("rootfs", &entries, &mut reader, &config()).expect("build image");

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
