//! Architecture resolution helpers.

use esp::arch::Arch as EspArch;
use koci::arch::Arch;

use crate::error::{Result, WizardError};

/// Converts a koci OCI architecture to the ESP model architecture.
#[must_use]
pub fn esp(arch: Arch) -> EspArch {
    match arch {
        Arch::Amd64 => EspArch::X86_64,
        Arch::Arm64 => EspArch::Aarch64,
        Arch::Riscv64 => EspArch::Riscv64,
    }
}

/// Parses an architecture name into its canonical value.
///
/// # Errors
///
/// Returns an error when the name is not a supported architecture.
pub fn parse(name: &str) -> Result<Arch> {
    match name {
        "amd64" => Ok(Arch::Amd64),
        "arm64" => Ok(Arch::Arm64),
        "riscv64" => Ok(Arch::Riscv64),
        other => Err(WizardError::ProfileValidation(format!(
            "unknown architecture: {other}"
        ))),
    }
}
