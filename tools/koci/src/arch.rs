//! Architecture identifiers of OCI images.

/// Target CPU architecture of an OCI image.
pub type Arch = oci::arch::Arch;

/// Architecture of the machine running the program.
#[must_use]
pub fn host() -> Arch {
    oci::arch::host()
}
