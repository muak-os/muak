//! ESP manifest model types.

use alloc::collections::BTreeSet;
use alloc::format;
use core::fmt;
use std::io::Read;

use fatfs::types::FileSource;

use crate::error::Result;
use crate::{error::EspError, path};

/// The target architecture determining the EFI fallback boot filename.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Arch {
    /// 64-bit x86 architecture.
    X86_64,
    /// 64-bit ARM architecture.
    Aarch64,
}

impl Arch {
    /// Returns the current compilation target architecture.
    #[must_use]
    pub const fn current() -> Self {
        #[cfg(target_arch = "x86_64")]
        {
            Self::X86_64
        }
        #[cfg(target_arch = "aarch64")]
        {
            Self::Aarch64
        }
        #[cfg(not(any(target_arch = "x86_64", target_arch = "aarch64")))]
        panic!("unsupported target architecture")
    }

    /// Returns the UEFI fallback boot filename for this architecture.
    #[must_use]
    pub const fn boot_filename(self) -> &'static str {
        match self {
            Self::X86_64 => "BOOTX64.EFI",
            Self::Aarch64 => "BOOTAA64.EFI",
        }
    }
}

/// A single file placed into the ESP at the given relative path.
pub struct EspFile {
    /// Relative path within the ESP.
    pub path: String,
    /// Readable stream for the file content.
    pub reader: Box<dyn Read>,
    /// Exact byte length of the content.
    pub size: u64,
}

impl fmt::Debug for EspFile {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("EspFile")
            .field("path", &self.path)
            .field("size", &self.size)
            .finish_non_exhaustive()
    }
}

impl EspFile {
    /// Creates an `EspFile` at the UEFI fallback boot path for `arch`.
    pub fn boot<R: Read + 'static>(arch: Arch, reader: R, size: u64) -> Self {
        Self {
            path: format!("EFI/BOOT/{}", arch.boot_filename()),
            reader: Box::new(reader),
            size,
        }
    }
}

impl FileSource for EspFile {
    fn path(&self) -> &str {
        &self.path
    }

    fn size(&self) -> u64 {
        self.size
    }

    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        self.reader.read(buf)
    }
}

/// Describes the complete file layout of an EFI System Partition.
pub struct EspSpec {
    files: Vec<EspFile>,
}

impl EspSpec {
    /// Returns a new builder for assembling a validated spec.
    #[must_use]
    pub fn builder() -> EspSpecBuilder {
        EspSpecBuilder::default()
    }

    /// Returns the validated, deduplicated list of files.
    #[must_use]
    pub fn files(&self) -> &[EspFile] {
        &self.files
    }

    /// Returns a mutable view of the validated file list for streaming consumers.
    pub fn files_mut(&mut self) -> &mut [EspFile] {
        &mut self.files
    }

    /// Returns path and size pairs for each file, for size pre-computation.
    pub fn metas(&self) -> impl Iterator<Item = (&str, u64)> {
        self.files
            .iter()
            .map(|file| (file.path.as_str(), file.size))
    }
}

/// Builds a validated `EspSpec` incrementally.
#[derive(Default)]
pub struct EspSpecBuilder {
    files: Vec<EspFile>,
    paths: BTreeSet<String>,
}

impl EspSpecBuilder {
    /// Adds one validated ESP file.
    ///
    /// # Errors
    ///
    /// Returns an error when the filepath is invalid or duplicates an existing normalized destination path.
    pub fn add_file(mut self, file: EspFile) -> Result<Self> {
        let normalized_path = path::normalize_relative_path(&file.path)?;
        if !self.paths.insert(normalized_path.clone()) {
            return Err(EspError::InvalidPath(format!(
                "duplicate ESP destination path: {normalized_path}"
            )));
        }
        self.files.push(EspFile {
            path: normalized_path,
            reader: file.reader,
            size: file.size,
        });

        Ok(self)
    }

    /// Adds multiple validated ESP files.
    ///
    /// # Errors
    ///
    /// Returns an error when any filepath is invalid or duplicates an existing normalized destination path.
    pub fn add_files<I: IntoIterator<Item = EspFile>>(self, files: I) -> Result<Self> {
        files.into_iter().try_fold(self, Self::add_file)
    }

    /// Finalizes the builder into an `EspSpec`.
    ///
    /// # Errors
    ///
    /// Returns an error when any filepath is invalid or not normalized.
    pub fn build(self) -> Result<EspSpec> {
        let spec = EspSpec { files: self.files };
        path::validate_spec(&spec)?;

        Ok(spec)
    }
}

impl fmt::Debug for EspSpecBuilder {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("EspSpecBuilder")
            .field("files", &self.files)
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;

    use super::{Arch, EspFile, EspSpec};
    use crate::error::EspError;

    fn dummy_file(path: &str, data: &[u8]) -> EspFile {
        EspFile {
            path: path.to_owned(),
            reader: Box::new(Cursor::new(data.to_vec())),
            size: u64::try_from(data.len()).unwrap_or(u64::MAX),
        }
    }

    #[test]
    fn arch_boot_filename_matches_uefi_fallback_names() {
        // ARRANGE / ACT / ASSERT
        assert_eq!(Arch::X86_64.boot_filename(), "BOOTX64.EFI");
        assert_eq!(Arch::Aarch64.boot_filename(), "BOOTAA64.EFI");
    }

    #[test]
    fn arch_current_matches_compilation_target() {
        // ARRANGE / ACT
        let arch = Arch::current();

        // ASSERT
        #[cfg(target_arch = "x86_64")]
        assert_eq!(arch, Arch::X86_64);
        #[cfg(target_arch = "aarch64")]
        assert_eq!(arch, Arch::Aarch64);
    }

    #[test]
    fn boot_file_creates_correct_path() {
        // ARRANGE
        let size = 1024_u64;
        let data = vec![0xAB; usize::try_from(size).unwrap_or(0)];

        // ACT
        let file = EspFile::boot(Arch::X86_64, Cursor::new(data), size);

        // ASSERT
        assert_eq!(file.path, "EFI/BOOT/BOOTX64.EFI");
        assert_eq!(file.size, size);
    }

    #[test]
    fn builder_add_file_places_path_first() {
        // ARRANGE / ACT
        let spec = EspSpec::builder()
            .add_file(dummy_file("EFI/BOOT/BOOTX64.EFI", b"uki"))
            .expect("file must be added")
            .build()
            .expect("spec must build");

        // ASSERT
        assert_eq!(spec.files().len(), 1);
    }

    #[test]
    fn builder_rejects_duplicate_normalized_paths() {
        // ARRANGE
        let builder = EspSpec::builder().add_file(dummy_file("EFI/BOOT/BOOTX64.EFI", b"first"));

        // ACT
        let second = dummy_file("./EFI/BOOT/BOOTX64.EFI", b"second");
        let result = builder.and_then(|builder| builder.add_file(second));

        // ASSERT
        assert!(matches!(result, Err(EspError::InvalidPath(_))));
    }

    #[test]
    fn builder_normalizes_curdir_components() {
        // ARRANGE / ACT
        let spec = EspSpec::builder()
            .add_file(dummy_file("./nested/file.txt", b"x"))
            .expect("file must be added")
            .build()
            .expect("spec must build");

        // ASSERT
        assert_eq!(
            spec.files().first().expect("file must exist").path,
            "nested/file.txt"
        );
    }

    #[test]
    fn builder_add_files_adds_all_entries() {
        // ARRANGE
        let files = vec![
            dummy_file("first.txt", b"a"),
            dummy_file("second.txt", b"bb"),
        ];

        // ACT
        let spec = EspSpec::builder()
            .add_files(files)
            .expect("files must be added")
            .build()
            .expect("spec must build");

        // ASSERT
        let collected: Vec<&str> = spec.files().iter().map(|file| file.path.as_str()).collect();
        assert_eq!(collected, ["first.txt", "second.txt"]);
    }

    #[test]
    fn spec_metas_projects_path_and_size() {
        // ARRANGE
        let spec = EspSpec::builder()
            .add_file(dummy_file("a.txt", b"hello"))
            .expect("file must be added")
            .build()
            .expect("spec must build");

        // ACT
        let metas: Vec<_> = spec.metas().collect();

        // ASSERT
        let (path, size) = metas.first().copied().expect("meta must exist");
        assert_eq!(path, "a.txt");
        assert_eq!(size, 5);
    }
}
