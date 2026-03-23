//! OCI image structures and utilities.

use serde::Deserialize;

/// OCI Manifest structure (platform-specific image manifest)
#[derive(Deserialize)]
pub struct OciManifest {
    #[serde(default)]
    pub layers: Vec<OciDescriptor>,
    #[serde(default)]
    pub manifests: Vec<OciDescriptor>,
}

/// OCI Descriptor (blob reference)
#[derive(Deserialize)]
pub struct OciDescriptor {
    pub digest: String,
    #[serde(default)]
    pub platform: Option<Platform>,
}

/// Platform information for multi-architecture images
#[derive(Deserialize, Default)]
pub struct Platform {
    pub architecture: Option<String>,
    pub os: Option<String>,
}

/// Image reference parser and utilities
#[derive(Debug, Clone)]
pub struct ImageReference {
    pub registry: String,
    pub name: String,
    pub tag: String,
}

impl ImageReference {
    /// Parse an image reference string into an ImageReference.
    pub fn parse(reference: &str) -> Self {
        let (reference, tag) = match reference.rsplit_once(':') {
            Some((r, t)) if !t.contains('/') => (r, t.to_string()),
            _ => (reference, "latest".to_string()),
        };

        let parts: Vec<&str> = reference.splitn(2, '/').collect();
        if parts.len() == 2 && (parts[0].contains('.') || parts[0].contains(':')) {
            let registry = match parts[0] {
                "docker.io" => "registry-1.docker.io",
                other => other,
            };
            Self {
                registry: registry.to_string(),
                name: parts[1].to_string(),
                tag,
            }
        } else {
            Self {
                registry: "registry-1.docker.io".to_string(),
                name: reference.to_string(),
                tag,
            }
        }
    }

    /// Determine the URL scheme (http or https) for the registry.
    pub fn scheme(&self) -> &'static str {
        if self.registry.starts_with("192.168.")
            || self.registry.starts_with("10.")
            || self.registry.starts_with("172.")
            || self.registry.starts_with("127.")
            || self.registry.starts_with("localhost")
        {
            "http"
        } else {
            "https"
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_docker_hub_image() {
        // ARRANGE
        let reference = "alpine:latest";

        // ACT
        let img = ImageReference::parse(reference);

        // ASSERT
        assert_eq!(img.registry, "registry-1.docker.io");
        assert_eq!(img.name, "alpine");
        assert_eq!(img.tag, "latest");
    }

    #[test]
    fn parse_ghcr_image() {
        // ARRANGE
        let reference = "ghcr.io/org/image:v1.0";

        // ACT
        let img = ImageReference::parse(reference);

        // ASSERT
        assert_eq!(img.registry, "ghcr.io");
        assert_eq!(img.name, "org/image");
        assert_eq!(img.tag, "v1.0");
    }

    #[test]
    fn parse_private_registry() {
        // ARRANGE
        let reference = "192.168.1.100:5000/myimage:tag";

        // ACT
        let img = ImageReference::parse(reference);

        // ASSERT
        assert_eq!(img.registry, "192.168.1.100:5000");
        assert_eq!(img.name, "myimage");
        assert_eq!(img.tag, "tag");
        assert_eq!(img.scheme(), "http");
    }

    #[test]
    fn parse_image_no_tag() {
        // ARRANGE
        let reference = "alpine";

        // ACT
        let img = ImageReference::parse(reference);

        // ASSERT
        assert_eq!(img.registry, "registry-1.docker.io");
        assert_eq!(img.name, "alpine");
        assert_eq!(img.tag, "latest");
    }

    #[test]
    fn parse_image_with_namespace() {
        // ARRANGE
        let reference = "library/alpine:3.14";

        // ACT
        let img = ImageReference::parse(reference);

        // ASSERT
        assert_eq!(img.registry, "registry-1.docker.io");
        assert_eq!(img.name, "library/alpine");
        assert_eq!(img.tag, "3.14");
    }

    #[test]
    fn parse_image_empty_string() {
        // ARRANGE
        let reference = "";

        // ACT
        let img = ImageReference::parse(reference);

        // ASSERT
        assert_eq!(img.tag, "latest");
    }

    #[test]
    fn parse_image_invalid_registry() {
        // ARRANGE
        let reference = "invalid@registry.com/image:tag";

        // ACT
        let img = ImageReference::parse(reference);

        // ASSERT
        assert_eq!(img.tag, "tag");
    }

    #[test]
    fn scheme_https_for_docker_io() {
        // ARRANGE
        let reference = "alpine:latest";

        // ACT
        let img = ImageReference::parse(reference);

        // ASSERT
        assert_eq!(img.scheme(), "https");
    }

    #[test]
    fn scheme_http_for_private() {
        // ARRANGE
        let reference = "192.168.1.1:5000/image:tag";

        // ACT
        let img = ImageReference::parse(reference);

        // ASSERT
        assert_eq!(img.scheme(), "http");
    }
}
