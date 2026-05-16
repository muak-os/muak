//! ESP manifest model types.

/// The target architecture determining the EFI fallback boot filename.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Arch {
    X86_64,
    Aarch64,
}

impl Arch {
    /// Returns the current compilation target architecture.
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
    pub fn with_uki(arch: Arch, uki: Vec<u8>, extra_files: Vec<EspFile>) -> Self {
        let mut files = Vec::with_capacity(1 + extra_files.len());
        files.push(EspFile {
            path: format!("EFI/BOOT/{}", arch.boot_filename()),
            data: uki,
        });
        files.extend(extra_files);
        Self { files }
    }

    /// Returns the total byte size of all file payloads in the spec.
    pub fn total_file_bytes(&self) -> usize {
        self.files.iter().map(|file| file.data.len()).sum()
    }
}

#[cfg(test)]
mod tests {
    use super::{Arch, EspFile, EspSpec};

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
        assert_eq!(spec.files[0].path, "EFI/BOOT/BOOTAA64.EFI");
        assert_eq!(spec.files[0].data, b"uki");
        assert_eq!(spec.files[1], extra_file);
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
}
