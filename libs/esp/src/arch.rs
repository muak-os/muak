//! ESP architecture.

/// The target architecture determining the EFI fallback boot filename.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Arch {
    /// 64-bit x86 architecture.
    X86_64,
    /// 64-bit ARM architecture.
    Aarch64,
    /// 64-bit RISC-V architecture.
    Riscv64,
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
        #[cfg(target_arch = "riscv64")]
        {
            Self::Riscv64
        }
        #[cfg(not(any(
            target_arch = "x86_64",
            target_arch = "aarch64",
            target_arch = "riscv64"
        )))]
        panic!("unsupported target architecture")
    }

    /// Returns the UEFI fallback boot path for this architecture.
    #[must_use]
    pub const fn boot_path(self) -> &'static str {
        match self {
            Self::X86_64 => "EFI/BOOT/BOOTX64.EFI",
            Self::Aarch64 => "EFI/BOOT/BOOTAA64.EFI",
            Self::Riscv64 => "EFI/BOOT/BOOTRISCV64.EFI",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::Arch;

    #[test]
    fn arch_boot_path_matches_uefi_fallback_paths() {
        // ARRANGE / ACT / ASSERT
        assert_eq!(Arch::X86_64.boot_path(), "EFI/BOOT/BOOTX64.EFI");
        assert_eq!(Arch::Aarch64.boot_path(), "EFI/BOOT/BOOTAA64.EFI");
        assert_eq!(Arch::Riscv64.boot_path(), "EFI/BOOT/BOOTRISCV64.EFI");
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
        #[cfg(target_arch = "riscv64")]
        assert_eq!(arch, Arch::Riscv64);
    }
}
