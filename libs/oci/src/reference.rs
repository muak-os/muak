//! Parsed OCI image references.

/// A parsed OCI image reference: registry, repository, and tag or digest.
#[derive(Debug, Clone)]
pub struct Image {
    /// Registry host such as `ghcr.io`, normalized for Docker Hub.
    pub registry: String,
    /// Repository path such as `org/image`.
    pub name: String,
    /// Tag or `sha256:...` digest selecting the manifest.
    pub manifest_ref: String,
}

impl Image {
    /// Parse an image reference string into an [`Image`].
    #[must_use]
    pub fn parse(reference: &str) -> Self {
        let digest_ref = reference
            .rsplit_once('@')
            .filter(|&(_, digest_ref)| is_digest_reference(digest_ref))
            .map(|(name, digest_ref)| (name, digest_ref.to_owned()));
        let (repository, manifest_ref) = digest_ref.unwrap_or_else(|| parse_tag_ref(reference));

        if let Some((registry_candidate, image_name)) = repository
            .split_once('/')
            .filter(|&(candidate, _)| candidate.contains('.') || candidate.contains(':'))
        {
            return Self {
                registry: normalize_registry(registry_candidate).to_owned(),
                name: image_name.to_owned(),
                manifest_ref,
            };
        }

        Self {
            registry: "registry-1.docker.io".to_owned(),
            name: repository.to_owned(),
            manifest_ref,
        }
    }

    /// Determine the URL scheme (`http` or `https`) for the registry.
    #[must_use]
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

fn normalize_registry(registry: &str) -> &str {
    match registry {
        "docker.io" => "registry-1.docker.io",
        other => other,
    }
}

fn parse_tag_ref(reference: &str) -> (&str, String) {
    match reference.rsplit_once(':') {
        Some((repository, tag)) if !tag.contains('/') => (repository, tag.to_owned()),
        _ => (reference, "latest".to_owned()),
    }
}

fn is_digest_reference(reference: &str) -> bool {
    let Some((algorithm, encoded)) = reference.split_once(':') else {
        return false;
    };

    !algorithm.is_empty()
        && !encoded.is_empty()
        && algorithm
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit())
        && encoded
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'+' | b'='))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_docker_hub_image() {
        // ARRANGE
        let reference = "alpine:latest";

        // ACT
        let img = Image::parse(reference);

        // ASSERT
        assert_eq!(img.registry, "registry-1.docker.io");
        assert_eq!(img.name, "alpine");
        assert_eq!(img.manifest_ref, "latest");
    }

    #[test]
    fn parse_ghcr_image() {
        // ARRANGE
        let reference = "ghcr.io/org/image:v1.0";

        // ACT
        let img = Image::parse(reference);

        // ASSERT
        assert_eq!(img.registry, "ghcr.io");
        assert_eq!(img.name, "org/image");
        assert_eq!(img.manifest_ref, "v1.0");
    }

    #[test]
    fn parse_private_registry() {
        // ARRANGE
        let reference = "192.168.1.100:5000/myimage:tag";

        // ACT
        let img = Image::parse(reference);

        // ASSERT
        assert_eq!(img.registry, "192.168.1.100:5000");
        assert_eq!(img.name, "myimage");
        assert_eq!(img.manifest_ref, "tag");
        assert_eq!(img.scheme(), "http");
    }

    #[test]
    fn parse_image_no_tag() {
        // ARRANGE
        let reference = "alpine";

        // ACT
        let img = Image::parse(reference);

        // ASSERT
        assert_eq!(img.registry, "registry-1.docker.io");
        assert_eq!(img.name, "alpine");
        assert_eq!(img.manifest_ref, "latest");
    }

    #[test]
    fn parse_image_with_namespace() {
        // ARRANGE
        let reference = "library/alpine:3.14";

        // ACT
        let img = Image::parse(reference);

        // ASSERT
        assert_eq!(img.registry, "registry-1.docker.io");
        assert_eq!(img.name, "library/alpine");
        assert_eq!(img.manifest_ref, "3.14");
    }

    #[test]
    fn parse_image_empty_string() {
        // ARRANGE
        let reference = "";

        // ACT
        let img = Image::parse(reference);

        // ASSERT
        assert_eq!(img.manifest_ref, "latest");
    }

    #[test]
    fn parse_image_invalid_registry() {
        // ARRANGE
        let reference = "invalid@registry.com/image:tag";

        // ACT
        let img = Image::parse(reference);

        // ASSERT
        assert_eq!(img.manifest_ref, "tag");
    }

    #[test]
    fn parse_image_digest_reference() {
        // ARRANGE
        let reference = "ghcr.io/org/image@sha256:0123456789abcdef";

        // ACT
        let img = Image::parse(reference);

        // ASSERT
        assert_eq!(img.registry, "ghcr.io");
        assert_eq!(img.name, "org/image");
        assert_eq!(img.manifest_ref, "sha256:0123456789abcdef");
    }

    #[test]
    fn parse_docker_io_registry_alias() {
        // ARRANGE
        let reference = "docker.io/library/alpine:3.20";

        // ACT
        let img = Image::parse(reference);

        // ASSERT
        assert_eq!(img.registry, "registry-1.docker.io");
        assert_eq!(img.name, "library/alpine");
        assert_eq!(img.manifest_ref, "3.20");
    }

    #[test]
    fn parse_digest_reference_without_separator_falls_back_to_tag() {
        // ARRANGE
        let reference = "ghcr.io/org/image@sha256";

        // ACT
        let img = Image::parse(reference);

        // ASSERT
        assert_eq!(img.registry, "ghcr.io");
        assert_eq!(img.name, "org/image@sha256");
        assert_eq!(img.manifest_ref, "latest");
    }

    #[test]
    fn parse_digest_reference_rejects_uppercase_algorithm() {
        // ARRANGE
        let reference = "ghcr.io/org/image@SHA256:abcdef";

        // ACT
        let img = Image::parse(reference);

        // ASSERT
        assert_eq!(img.name, "org/image@SHA256");
        assert_eq!(img.manifest_ref, "abcdef");
    }

    #[test]
    fn parse_digest_reference_with_empty_encoded_value_falls_back_to_tag_parse() {
        // ARRANGE
        let reference = "ghcr.io/org/image@sha256:";

        // ACT
        let img = Image::parse(reference);

        // ASSERT
        assert_eq!(img.name, "org/image@sha256");
        assert_eq!(img.manifest_ref, "");
    }

    #[test]
    fn scheme_https_for_docker_io() {
        // ARRANGE
        let reference = "alpine:latest";

        // ACT
        let img = Image::parse(reference);

        // ASSERT
        assert_eq!(img.scheme(), "https");
    }

    #[test]
    fn scheme_http_for_private() {
        // ARRANGE
        let reference = "192.168.1.1:5000/image:tag";

        // ACT
        let img = Image::parse(reference);

        // ASSERT
        assert_eq!(img.scheme(), "http");
    }

    #[test]
    fn scheme_http_for_localhost_registry() {
        // ARRANGE
        let reference = "localhost:5000/repo:tag";

        // ACT
        let img = Image::parse(reference);

        // ASSERT
        assert_eq!(img.scheme(), "http");
    }
}
