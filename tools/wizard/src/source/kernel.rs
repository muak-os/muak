//! Kernel OCI source reference.

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolved_kernel_accessors() {
        // ARRANGE
        let kernel = Kernel::new(
            "ghcr.io/muak-os/kernel".into(),
            "ghcr.io/muak-os/kernel:v1.0.0".into(),
        );

        // ACT & ASSERT
        assert_eq!(kernel.image(), "ghcr.io/muak-os/kernel");
        assert_eq!(kernel.source(), "ghcr.io/muak-os/kernel:v1.0.0");
    }
}
