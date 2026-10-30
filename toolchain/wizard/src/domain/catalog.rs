//! Catalog image identity used by a resolution.

/// The core catalog image a resolution was derived from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Catalog {
    image: String,
    digest: String,
}

impl Catalog {
    #[must_use]
    pub(crate) fn new(image: String, digest: String) -> Self {
        Self { image, digest }
    }

    /// Returns the catalog repository without a tag.
    #[must_use]
    pub fn image(&self) -> &str {
        &self.image
    }

    /// Returns the digest of the catalog manifest.
    #[must_use]
    pub fn digest(&self) -> &str {
        &self.digest
    }

    /// Returns the digest-pinned catalog reference.
    #[must_use]
    pub fn pinned(&self) -> String {
        format!("{}@{}", self.image, self.digest)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catalog_exposes_its_image_digest_and_pin() {
        // ARRANGE
        let catalog = Catalog::new("ghcr.io/muak-os/core".into(), "sha256:abcd".into());

        // ACT
        let image = catalog.image();
        let digest = catalog.digest();
        let pinned = catalog.pinned();

        // ASSERT
        assert_eq!(image, "ghcr.io/muak-os/core");
        assert_eq!(digest, "sha256:abcd");
        assert_eq!(pinned, "ghcr.io/muak-os/core@sha256:abcd");
    }
}
