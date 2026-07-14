//! ESP architecture.

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

#[cfg(test)]
mod tests {
    use super::Arch;

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
}
