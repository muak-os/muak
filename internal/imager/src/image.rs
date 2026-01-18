//! OCI image structures and utilities.

use serde::Deserialize;

/// OCI Index structure (top-level manifest list)
#[derive(Deserialize)]
pub struct OciIndex {
    pub manifests: Vec<OciDescriptor>,
}

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

    pub fn image_name(&self) -> String {
        self.name
            .split('/')
            .next_back()
            .unwrap_or("extension")
            .to_string()
    }

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
    fn test_parse_docker_hub_image() {
        let img = ImageReference::parse("alpine:latest");
        assert_eq!(img.registry, "registry-1.docker.io");
        assert_eq!(img.name, "alpine");
        assert_eq!(img.tag, "latest");
    }

    #[test]
    fn test_parse_ghcr_image() {
        let img = ImageReference::parse("ghcr.io/org/image:v1.0");
        assert_eq!(img.registry, "ghcr.io");
        assert_eq!(img.name, "org/image");
        assert_eq!(img.tag, "v1.0");
    }

    #[test]
    fn test_parse_private_registry() {
        let img = ImageReference::parse("192.168.1.100:5000/myimage:tag");
        assert_eq!(img.registry, "192.168.1.100:5000");
        assert_eq!(img.name, "myimage");
        assert_eq!(img.tag, "tag");
        assert_eq!(img.scheme(), "http");
    }

    #[test]
    fn test_image_name_extraction() {
        let img = ImageReference::parse("ghcr.io/org/my-extension:v1");
        assert_eq!(img.image_name(), "my-extension");
    }

    #[test]
    fn test_parse_image_no_tag() {
        let img = ImageReference::parse("alpine");
        assert_eq!(img.registry, "registry-1.docker.io");
        assert_eq!(img.name, "alpine");
        assert_eq!(img.tag, "latest");
    }

    #[test]
    fn test_parse_image_with_namespace() {
        let img = ImageReference::parse("library/alpine:3.14");
        assert_eq!(img.registry, "registry-1.docker.io");
        assert_eq!(img.name, "library/alpine");
        assert_eq!(img.tag, "3.14");
    }

    #[test]
    fn test_parse_image_empty_string() {
        let img = ImageReference::parse("");
        // Should handle gracefully, perhaps default to latest
        assert_eq!(img.tag, "latest");
    }

    #[test]
    fn test_parse_image_invalid_registry() {
        let img = ImageReference::parse("invalid@registry.com/image:tag");
        // Parsing might still work if it splits on /
        // But let's check it doesn't panic
        assert_eq!(img.tag, "tag");
    }

    #[test]
    fn test_scheme_https_for_docker_io() {
        let img = ImageReference::parse("alpine:latest");
        assert_eq!(img.scheme(), "https");
    }

    #[test]
    fn test_scheme_http_for_private() {
        let img = ImageReference::parse("192.168.1.1:5000/image:tag");
        assert_eq!(img.scheme(), "http");
    }
}
