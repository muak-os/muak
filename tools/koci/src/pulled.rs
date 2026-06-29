//! In-memory pulled OCI image representation.

#![expect(
    clippy::module_name_repetitions,
    reason = "Public Pulled* types intentionally share the module prefix"
)]

extern crate alloc;

use alloc::collections::BTreeMap;
use alloc::sync::Arc;
use std::io::Cursor;
use std::path::{Path, PathBuf};

use crate::error::{KociError, Result};

const DEFAULT_DIR_MODE: u32 = 0o755;

#[derive(Debug, Clone)]
enum PulledNode {
    File { mode: u32, data: Arc<[u8]> },
    Dir { mode: u32 },
}

/// A file extracted from a pulled OCI image.
#[derive(Debug, Clone)]
pub struct PulledFile {
    /// File length in bytes.
    pub len: u64,
    /// File mode bits.
    pub mode: u32,
    /// Opaque file contents retained by the pulled image.
    #[doc(hidden)]
    pub data: Arc<[u8]>,
}

impl PulledFile {
    /// Open a readable stream for this file.
    #[must_use]
    pub fn open(&self) -> Cursor<Arc<[u8]>> {
        Cursor::new(Arc::clone(&self.data))
    }
}

/// A single entry in a pulled OCI image.
#[derive(Debug, Clone)]
pub enum PulledEntry {
    /// A regular file entry.
    File {
        /// Entry path relative to the image root.
        path: PathBuf,
        /// File metadata and content handle.
        file: PulledFile,
    },
    /// A directory entry.
    Dir {
        /// Entry path relative to the image root.
        path: PathBuf,
        /// Directory mode bits.
        mode: u32,
    },
}

impl PulledEntry {
    /// Path of this entry relative to the image root.
    #[must_use]
    pub fn path(&self) -> &Path {
        match *self {
            Self::File { ref path, .. } | Self::Dir { ref path, .. } => path.as_path(),
        }
    }
}

/// A pulled OCI image materialized as an in-memory merged filesystem view.
#[derive(Debug, Clone, Default)]
pub struct PulledImage {
    entries: BTreeMap<PathBuf, PulledNode>,
}

impl PulledImage {
    /// Create an empty pulled image.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Look up a file by path.
    ///
    /// # Errors
    ///
    /// This method currently cannot fail.
    pub fn file(&self, path: &Path) -> Result<Option<PulledFile>> {
        Ok(match self.entries.get(path) {
            Some(&PulledNode::File { mode, ref data }) => Some(PulledFile {
                len: u64::try_from(data.len()).unwrap_or(u64::MAX),
                mode,
                data: Arc::clone(data),
            }),
            _ => None,
        })
    }

    /// List all entries in deterministic path order.
    ///
    /// # Errors
    ///
    /// This method currently cannot fail.
    pub fn entries(&self) -> Result<Vec<PulledEntry>> {
        Ok(self
            .entries
            .iter()
            .map(|(path, node)| match *node {
                PulledNode::File { mode, ref data } => PulledEntry::File {
                    path: path.clone(),
                    file: PulledFile {
                        len: u64::try_from(data.len()).unwrap_or(u64::MAX),
                        mode,
                        data: Arc::clone(data),
                    },
                },
                PulledNode::Dir { mode } => PulledEntry::Dir {
                    path: path.clone(),
                    mode,
                },
            })
            .collect())
    }

    /// Materialize the pulled image to a directory on disk.
    ///
    /// # Errors
    ///
    /// Returns an error if directories or files cannot be written.
    pub fn write_to_dir(&self, output: &Path) -> Result<()> {
        std::fs::create_dir_all(output).map_err(|source| KociError::WriteError {
            file: output.display().to_string(),
            source,
        })?;

        for entry in self.entries()? {
            write_entry_to_dir(output, entry)?;
        }

        Ok(())
    }

    /// Add a directory entry to the pulled image (public for test convenience).
    pub fn add_dir(&mut self, path: &Path, mode: u32) {
        self.insert_dir(path, mode);
    }

    /// Add a file entry to the pulled image (public for test convenience).
    pub fn add_file(&mut self, path: &Path, mode: u32, data: Vec<u8>) {
        self.insert_file(path, mode, data);
    }

    pub(crate) fn insert_dir(&mut self, path: &Path, mode: u32) {
        if path.as_os_str().is_empty() {
            return;
        }

        self.ensure_parent_dirs(path);
        self.entries
            .insert(path.to_path_buf(), PulledNode::Dir { mode });
    }

    pub(crate) fn insert_file(&mut self, path: &Path, mode: u32, data: Vec<u8>) {
        self.ensure_parent_dirs(path);
        self.entries.insert(
            path.to_path_buf(),
            PulledNode::File {
                mode,
                data: Arc::<[u8]>::from(data),
            },
        );
    }

    pub(crate) fn remove_path(&mut self, path: &Path) {
        if path.as_os_str().is_empty() {
            self.entries.clear();
            return;
        }

        self.entries.remove(path);
        self.entries
            .retain(|candidate, _| !candidate.starts_with(path));
    }

    fn ensure_parent_dirs(&mut self, path: &Path) {
        let mut current = PathBuf::new();
        let Some(parent) = path.parent() else {
            return;
        };

        for component in parent.components() {
            current.push(component.as_os_str());
            self.entries
                .entry(current.clone())
                .or_insert(PulledNode::Dir {
                    mode: DEFAULT_DIR_MODE,
                });
        }
    }
}

fn write_entry_to_dir(output: &Path, entry: PulledEntry) -> Result<()> {
    match entry {
        PulledEntry::Dir { path, .. } => {
            let dir_path = output.join(path);
            std::fs::create_dir_all(&dir_path).map_err(|source| KociError::WriteError {
                file: dir_path.display().to_string(),
                source,
            })
        }
        PulledEntry::File { path, file } => {
            let output_path = output.join(path);
            if let Some(parent) = output_path.parent() {
                std::fs::create_dir_all(parent).map_err(|source| KociError::WriteError {
                    file: parent.display().to_string(),
                    source,
                })?;
            }
            let mut reader = file.open();
            let mut writer =
                std::fs::File::create(&output_path).map_err(|source| KociError::WriteError {
                    file: output_path.display().to_string(),
                    source,
                })?;
            std::io::copy(&mut reader, &mut writer).map_err(|source| KociError::WriteError {
                file: output_path.display().to_string(),
                source,
            })?;
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use std::io::Read as _;

    use super::*;

    #[test]
    fn pulled_image_file_lookup_returns_openable_file() {
        // ARRANGE
        let mut image = PulledImage::new();
        image.insert_file(Path::new("etc/motd"), 0o644, b"hello\n".to_vec());

        // ACT
        let file = image
            .file(Path::new("etc/motd"))
            .expect("file lookup")
            .expect("missing file");
        let mut reader = file.open();
        let mut buf = String::new();
        reader.read_to_string(&mut buf).expect("read file");

        // ASSERT
        assert_eq!(file.len, 6);
        assert_eq!(file.mode, 0o644);
        assert_eq!(buf, "hello\n");
    }

    #[test]
    fn pulled_image_entries_include_parent_directories() {
        // ARRANGE
        let mut image = PulledImage::new();
        image.insert_file(Path::new("usr/share/doc/readme"), 0o644, b"docs".to_vec());

        // ACT
        let entries = image.entries().expect("entries");
        let paths: Vec<PathBuf> = entries
            .iter()
            .map(|entry| entry.path().to_path_buf())
            .collect();

        // ASSERT
        assert!(paths.contains(&PathBuf::from("usr")));
        assert!(paths.contains(&PathBuf::from("usr/share")));
        assert!(paths.contains(&PathBuf::from("usr/share/doc")));
        assert!(paths.contains(&PathBuf::from("usr/share/doc/readme")));
    }

    #[test]
    fn pulled_image_remove_path_removes_descendants() {
        // ARRANGE
        let mut image = PulledImage::new();
        image.insert_file(Path::new("etc/conf.d/a"), 0o644, b"a".to_vec());
        image.insert_file(Path::new("etc/conf.d/b"), 0o644, b"b".to_vec());

        // ACT
        image.remove_path(Path::new("etc/conf.d"));

        // ASSERT
        assert!(
            image
                .file(Path::new("etc/conf.d/a"))
                .expect("file lookup")
                .is_none()
        );
        assert!(
            image
                .file(Path::new("etc/conf.d/b"))
                .expect("file lookup")
                .is_none()
        );
    }
}
