//! Architecture resolution helpers.

use esp::model::Arch as EspArch;
use koci::arch::Arch;

/// Converts a koci OCI architecture to the ESP model architecture.
#[must_use]
pub fn esp(arch: Arch) -> EspArch {
    match arch {
        Arch::Amd64 => EspArch::X86_64,
        Arch::Arm64 => EspArch::Aarch64,
    }
}
