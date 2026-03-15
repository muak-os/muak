//! Semver-aware version comparison for image tags.

use crate::error::{ConfigError, Result};

/// Parsed representation of an image tag.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ImageVersion {
    Latest,
    Semver(u64, u64, u64),
}

impl ImageVersion {
    /// Returns `true` if `self` is strictly older than `other`.
    pub fn is_downgrade_from(&self, current: &ImageVersion) -> bool {
        match (self, current) {
            (ImageVersion::Latest, _) | (_, ImageVersion::Latest) => false,
            (ImageVersion::Semver(ma, mi, pa), ImageVersion::Semver(mb, mib, pb)) => {
                (ma, mi, pa) < (mb, mib, pb)
            }
        }
    }
}

/// Extracts the tag portion from an image reference.
fn extract_tag(image: &str) -> &str {
    let image = match image.find('@') {
        Some(at) => &image[..at],
        None => image,
    };
    if let Some(colon_pos) = image.rfind(':') {
        let after_colon = &image[colon_pos + 1..];
        if !after_colon.contains('/') {
            return after_colon;
        }
    }
    ""
}

/// Parses the tag of an image reference into an [`ImageVersion`].
pub fn parse_image_version(image: &str) -> Result<ImageVersion> {
    let tag = extract_tag(image);
    parse_tag(tag)
}

fn parse_tag(tag: &str) -> Result<ImageVersion> {
    if tag.is_empty() || tag == "latest" {
        return Ok(ImageVersion::Latest);
    }

    let s = tag.strip_prefix('v').ok_or_else(|| {
        ConfigError::ValidationError(format!(
            "image tag '{}' is not a valid version (expected 'latest' or 'vX.X.X')",
            tag
        ))
    })?;

    let parts: Vec<&str> = s.split('.').collect();
    if parts.len() != 3 {
        return Err(ConfigError::ValidationError(format!(
            "image tag '{}' is not a valid version (expected 'latest' or 'vX.X.X')",
            tag
        )));
    }

    let parse = |p: &str| {
        p.parse::<u64>().map_err(|_| {
            ConfigError::ValidationError(format!(
                "image tag '{}' contains non-numeric component '{}'",
                tag, p
            ))
        })
    };

    Ok(ImageVersion::Semver(
        parse(parts[0])?,
        parse(parts[1])?,
        parse(parts[2])?,
    ))
}

/// Validates that `new_image` is not a downgrade relative to `current_image`.
pub fn check_no_downgrade(new_image: &str, current_image: &str) -> Result<()> {
    let new_ver = parse_image_version(new_image)?;
    let current_ver = parse_image_version(current_image)?;

    if new_ver.is_downgrade_from(&current_ver) {
        let new_tag = extract_tag(new_image);
        let cur_tag = extract_tag(current_image);
        return Err(ConfigError::ValidationError(format!(
            "downgrade rejected: target version '{}' is older than installed version '{}'",
            new_tag, cur_tag
        )));
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extract_tag_parses_correctly() {
        // ACT & ASSERT
        assert_eq!(extract_tag("ghcr.io/foo/bar:v1.2.3"), "v1.2.3");
        assert_eq!(extract_tag("ghcr.io/foo/bar:latest"), "latest");
        assert_eq!(extract_tag("ghcr.io/foo/bar"), "");
        assert_eq!(
            extract_tag(
                "ghcr.io/foo/bar:v1.2.3@sha256:43be85a382dd4f2cf7fb52db998730f9d6da8391ba05d179ba1cabca3a8c2b20"
            ),
            "v1.2.3"
        );
        assert_eq!(extract_tag("localhost:5000/bar:v0.1.0"), "v0.1.0");
        assert_eq!(extract_tag("localhost:5000/bar"), "");
    }

    #[test]
    fn parse_versions() {
        // ACT & ASSERT
        assert_eq!(
            parse_image_version("img:latest").unwrap(),
            ImageVersion::Latest
        );
        assert_eq!(parse_image_version("img").unwrap(), ImageVersion::Latest);
        assert_eq!(
            parse_image_version("img:v1.2.3").unwrap(),
            ImageVersion::Semver(1, 2, 3)
        );
        assert_eq!(
            parse_image_version(
                "img:v1.2.3@sha256:43be85a382dd4f2cf7fb52db998730f9d6da8391ba05d179ba1cabca3a8c2b20"
            )
            .unwrap(),
            ImageVersion::Semver(1, 2, 3)
        );
        assert!(parse_image_version("img:1.2.3").is_err());
        assert!(parse_image_version("img:v1.2").is_err());
        assert!(parse_image_version("img:v1.2.x").is_err());
    }

    #[test]
    fn downgrade_detection() {
        // ACT & ASSERT
        assert!(check_no_downgrade("img:v0.1.0", "img:v0.2.0").is_err());
        assert!(check_no_downgrade("img:v1.0.0", "img:v2.0.0").is_err());
        assert!(check_no_downgrade("img:v1.2.3", "img:v1.2.4").is_err());

        assert!(check_no_downgrade("img:v1.2.3", "img:v1.2.3").is_ok());

        assert!(check_no_downgrade("img:v1.3.0", "img:v1.2.9").is_ok());
        assert!(check_no_downgrade("img:v2.0.0", "img:v1.99.99").is_ok());

        assert!(check_no_downgrade("img:latest", "img:v99.0.0").is_ok());
        assert!(check_no_downgrade("img:latest", "img:latest").is_ok());
        assert!(check_no_downgrade("img:v1.0.0", "img:latest").is_ok());
    }
}
