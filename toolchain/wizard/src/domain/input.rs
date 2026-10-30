//! Resolved inputs selected by a profile.

use koci::arch::Arch;

/// A kernel source resolved from the profile.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Kernel {
    /// Fully-qualified kernel image path.
    image: String,
    /// Versioned OCI reference for the kernel image.
    source: String,
}

impl Kernel {
    #[must_use]
    pub(crate) fn new(image: String, source: String) -> Self {
        Self { image, source }
    }

    /// Returns the fully-qualified kernel image path.
    #[must_use]
    pub fn image(&self) -> &str {
        &self.image
    }

    /// Returns the versioned OCI reference for this kernel image.
    #[must_use]
    pub fn source(&self) -> &str {
        &self.source
    }
}

/// A reference to an extension source.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Extension {
    /// Logical extension name.
    name: String,
    /// Versioned OCI reference for the extension source.
    source: String,
}

impl Extension {
    #[must_use]
    pub(crate) fn new(name: String, source: String) -> Self {
        Self { name, source }
    }

    /// Returns the canonical logical extension name.
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Returns the versioned OCI reference for this extension.
    #[must_use]
    pub fn source(&self) -> &str {
        &self.source
    }
}

/// An overlay source resolved from the profile.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Overlay {
    /// Overlay name inside the OCI image.
    pub name: String,
    /// Logical overlay image name.
    pub image: String,
    /// Versioned OCI reference for the overlay image.
    pub source: String,
    /// Target architecture of the overlay.
    pub arch: Arch,
}

impl Overlay {
    #[must_use]
    pub(crate) fn new(name: String, image: String, source: String, arch: Arch) -> Self {
        Self {
            name,
            image,
            source,
            arch,
        }
    }

    /// Returns the selected overlay name inside the OCI image.
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Returns the logical overlay image name.
    #[must_use]
    pub fn image(&self) -> &str {
        &self.image
    }

    /// Returns the versioned OCI reference for this overlay image.
    #[must_use]
    pub fn source_ref(&self) -> &str {
        &self.source
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolved_kernel_accessors() {
        // ARRANGE
        let kernel = Kernel::new(
            "ghcr.io/muak-os/linux".into(),
            "ghcr.io/muak-os/linux:v1.0.0".into(),
        );

        // ACT & ASSERT
        assert_eq!(kernel.image(), "ghcr.io/muak-os/linux");
        assert_eq!(kernel.source(), "ghcr.io/muak-os/linux:v1.0.0");
    }

    #[test]
    fn resolved_extension_accessors() {
        // ARRANGE
        let ext = Extension::new(
            "qemu".into(),
            "ghcr.io/muak-os/extensions/qemu:v1.0.0".into(),
        );

        // ACT & ASSERT
        assert_eq!(ext.name(), "qemu");
        assert_eq!(ext.source(), "ghcr.io/muak-os/extensions/qemu:v1.0.0");
    }

    #[test]
    fn resolved_overlay_accessors() {
        // ARRANGE
        let ov = Overlay::new(
            "rpi_generic".into(),
            "muak-os/sbc-raspberrypi".into(),
            "ghcr.io/muak-os/sbc/raspberrypi:v1.0.0".into(),
            Arch::Amd64,
        );

        // ACT & ASSERT
        assert_eq!(ov.name(), "rpi_generic");
        assert_eq!(ov.image(), "muak-os/sbc-raspberrypi");
        assert_eq!(ov.source_ref(), "ghcr.io/muak-os/sbc/raspberrypi:v1.0.0");
    }
}
