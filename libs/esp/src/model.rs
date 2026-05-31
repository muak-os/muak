//! ESP manifest model types.

use alloc::collections::BTreeSet;

use crate::error::Result;
use crate::{EspError, path};

/// The target architecture determining the EFI fallback boot filename.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Arch {
    X86_64,
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
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EspFile {
    pub path: String,
    pub data: Vec<u8>,
}

/// Describes the complete file layout of an EFI System Partition.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EspSpec {
    pub files: Vec<EspFile>,
}

impl EspSpec {
    /// Constructs an ESP spec from a UKI blob and additional files.
    #[must_use]
    pub fn with_uki(arch: Arch, uki: Vec<u8>, extra_files: Vec<EspFile>) -> Self {
        let mut files = Vec::with_capacity(extra_files.len().saturating_add(1));
        files.push(EspFile {
            path: format!("EFI/BOOT/{}", arch.boot_filename()),
            data: uki,
        });
        files.extend(extra_files);
        Self { files }
    }

    /// Returns a new builder for assembling a validated spec.
    #[must_use]
    pub fn builder() -> EspSpecBuilder {
        EspSpecBuilder::default()
    }

    /// Returns the total byte size of all file payloads in the spec.
    #[must_use]
    pub fn total_file_bytes(&self) -> usize {
        self.files.iter().map(|file| file.data.len()).sum()
    }
}

/// Builds a validated `EspSpec` incrementally.
#[derive(Debug, Default)]
pub struct EspSpecBuilder {
    files: Vec<EspFile>,
    paths: BTreeSet<String>,
}

impl EspSpecBuilder {
    /// Adds the fallback boot file for `arch` from the provided UKI bytes.
    ///
    /// # Errors
    ///
    /// Returns an error when the generated fallback boot path conflicts with an existing
    /// filepath in the builder.
    pub fn with_uki(self, arch: Arch, uki: Vec<u8>) -> Result<Self> {
        self.add_file(EspFile {
            path: format!("EFI/BOOT/{}", arch.boot_filename()),
            data: uki,
        })
    }

    /// Adds one validated ESP file.
    ///
    /// # Errors
    ///
    /// Returns an error when the filepath is invalid or duplicates an existing normalized
    /// destination path.
    pub fn add_file(mut self, file: EspFile) -> Result<Self> {
        let normalized_path = path::normalize_relative_path(&file.path)?;
        if !self.paths.insert(normalized_path.clone()) {
            return Err(EspError::InvalidPath(format!(
                "duplicate ESP destination path: {normalized_path}"
            )));
        }
        self.files.push(EspFile {
            path: normalized_path,
            data: file.data,
        });
        Ok(self)
    }

    /// Adds multiple validated ESP files.
    ///
    /// # Errors
    ///
    /// Returns an error when any filepath is invalid or duplicates an existing normalized
    /// destination path.
    pub fn add_files(self, files: Vec<EspFile>) -> Result<Self> {
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

#[cfg(test)]
mod tests {
    use super::{Arch, EspFile, EspSpec};
    use crate::EspError;

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
    fn with_uki_places_boot_file_first() {
        // ARRANGE
        let extra_file = EspFile {
            path: "config.txt".to_owned(),
            data: b"x".to_vec(),
        };

        // ACT
        let spec = EspSpec::with_uki(Arch::Aarch64, b"uki".to_vec(), vec![extra_file.clone()]);

        // ASSERT
        assert_eq!(spec.files.len(), 2);
        let boot_file = spec.files.first().expect("boot file must exist");
        let config_file = spec.files.get(1).expect("config file must exist");
        assert_eq!(boot_file.path, "EFI/BOOT/BOOTAA64.EFI");
        assert_eq!(boot_file.data, b"uki");
        assert_eq!(config_file, &extra_file);
    }

    #[test]
    fn total_file_bytes_sums_all_entries() {
        // ARRANGE
        let spec = EspSpec {
            files: vec![
                EspFile {
                    path: "a".to_owned(),
                    data: vec![1, 2, 3],
                },
                EspFile {
                    path: "b".to_owned(),
                    data: vec![4, 5],
                },
            ],
        };

        // ACT
        let total = spec.total_file_bytes();

        // ASSERT
        assert_eq!(total, 5);
    }

    #[test]
    fn builder_with_uki_places_boot_file_first() {
        // ARRANGE
        let builder = EspSpec::builder();

        // ACT
        let spec = builder
            .with_uki(Arch::X86_64, b"uki".to_vec())
            .expect("boot file must be added")
            .build()
            .expect("spec must build");

        // ASSERT
        assert_eq!(spec.files.len(), 1);
        let boot_file = spec.files.first().expect("boot file must exist");
        assert_eq!(boot_file.path, "EFI/BOOT/BOOTX64.EFI");
        assert_eq!(boot_file.data, b"uki");
    }

    #[test]
    fn builder_rejects_duplicate_normalized_paths() {
        // ARRANGE
        let builder = EspSpec::builder().add_file(EspFile {
            path: "EFI/BOOT/BOOTX64.EFI".to_owned(),
            data: b"first".to_vec(),
        });

        // ACT
        let result = builder.and_then(|builder| {
            builder.add_file(EspFile {
                path: "./EFI/BOOT/BOOTX64.EFI".to_owned(),
                data: b"second".to_vec(),
            })
        });

        // ASSERT
        assert!(matches!(result, Err(EspError::InvalidPath(_))));
    }

    #[test]
    fn builder_normalizes_curdir_components() {
        // ARRANGE
        let builder = EspSpec::builder();

        // ACT
        let spec = builder
            .add_file(EspFile {
                path: "./nested/file.txt".to_owned(),
                data: b"x".to_vec(),
            })
            .expect("file must be added")
            .build()
            .expect("spec must build");

        // ASSERT
        let file = spec.files.first().expect("file must exist");
        assert_eq!(file.path, "nested/file.txt");
    }

    #[test]
    fn builder_add_files_adds_all_entries() {
        // ARRANGE
        let files = vec![
            EspFile {
                path: "first.txt".to_owned(),
                data: b"a".to_vec(),
            },
            EspFile {
                path: "second.txt".to_owned(),
                data: b"bb".to_vec(),
            },
        ];

        // ACT
        let spec = EspSpec::builder()
            .add_files(files)
            .expect("files must be added")
            .build()
            .expect("spec must build");

        // ASSERT
        assert_eq!(spec.files.len(), 2);
        let first = spec.files.first().expect("first file must exist");
        let second = spec.files.get(1).expect("second file must exist");
        assert_eq!(first.path, "first.txt");
        assert_eq!(second.path, "second.txt");
    }
}
