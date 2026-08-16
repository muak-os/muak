//! Overlay OCI source references.

use koci::arch::Arch;

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
    fn resolved_overlay_accessors() {
        // ARRANGE
        let ov = Overlay::new(
            "rpi_generic".into(),
            "muak-os/sbc-raspberrypi".into(),
            "ghcr.io/muak-os/sbc-raspberrypi:v1.0.0".into(),
            Arch::Amd64,
        );

        // ACT & ASSERT
        assert_eq!(ov.name(), "rpi_generic");
        assert_eq!(ov.image(), "muak-os/sbc-raspberrypi");
        assert_eq!(ov.source_ref(), "ghcr.io/muak-os/sbc-raspberrypi:v1.0.0");
    }
}
