//! Architecture resolution helpers.

/// Converts a koci OCI architecture to the ESP model architecture.
#[must_use]
pub fn esp(arch: koci::arch::Arch) -> esp::model::Arch {
    match arch {
        koci::arch::Arch::Amd64 => esp::model::Arch::X86_64,
        koci::arch::Arch::Arm64 => esp::model::Arch::Aarch64,
    }
}
