//! Semver-aware version comparison for image tags and package versions.

use crate::error::{ConfigError, Result};

/// Parsed representation of a version.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Version {
    /// The `latest` tag, always considered newer than any semver.
    Latest,
    /// A semantic version with optional pre-release tag.
    Semver(u64, u64, u64, Option<String>),
}

impl Version {
    /// Returns `true` if `self` is strictly older than `other`.
    pub fn is_downgrade_from(&self, current: &Version) -> bool {
        match (self, current) {
            (Version::Latest, _) | (_, Version::Latest) => false,
            (Version::Semver(ma, mi, pa, pre_a), Version::Semver(mb, mib, pb, pre_b)) => {
                let core_a = (ma, mi, pa);
                let core_b = (mb, mib, pb);
                match core_a.cmp(&core_b) {
                    std::cmp::Ordering::Less => true,
                    std::cmp::Ordering::Greater => false,
                    std::cmp::Ordering::Equal => matches!((pre_a, pre_b), (Some(_), None)),
                }
            }
        }
    }

    /// Returns the major version component, or `None` for `Latest`.
    pub fn major(&self) -> Option<u64> {
        match self {
            Version::Semver(maj, _, _, _) => Some(*maj),
            Version::Latest => None,
        }
    }

    /// Returns the minor version component, or `None` for `Latest`.
    pub fn minor(&self) -> Option<u64> {
        match self {
            Version::Semver(_, min, _, _) => Some(*min),
            Version::Latest => None,
        }
    }
}

impl std::fmt::Display for Version {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Version::Latest => write!(f, "latest"),
            Version::Semver(maj, min, patch, None) => {
                write!(f, "{}.{}.{}", maj, min, patch)
            }
            Version::Semver(maj, min, patch, Some(pre)) => {
                write!(f, "{}.{}.{}-{}", maj, min, patch, pre)
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

/// Parses the tag of an image.
pub fn parse_image_version(image: &str) -> Result<Version> {
    let tag = extract_tag(image);
    parse_tag(tag)
}

/// Parses a bare `CARGO_PKG_VERSION`.
pub fn parse_pkg_version(version: &str) -> Result<Version> {
    parse_semver(version)
}

fn parse_tag(tag: &str) -> Result<Version> {
    if tag.is_empty() || tag == "latest" {
        return Ok(Version::Latest);
    }

    let s = tag.strip_prefix('v').ok_or_else(|| {
        ConfigError::ValidationError(format!(
            "image tag '{}' is not a valid version (expected 'latest' or 'vX.X.X[-pre]')",
            tag
        ))
    })?;

    parse_semver(s).map_err(|_| {
        ConfigError::ValidationError(format!(
            "image tag '{}' is not a valid version (expected 'latest' or 'vX.X.X[-pre]')",
            tag
        ))
    })
}

fn parse_semver(s: &str) -> Result<Version> {
    let (core, pre) = match s.split_once('-') {
        Some((c, p)) => (c, Some(p.to_owned())),
        None => (s, None),
    };

    let parts: Vec<&str> = core.split('.').collect();
    if parts.len() != 3 {
        return Err(ConfigError::ValidationError(format!(
            "'{}' is not a valid semver (expected X.X.X or X.X.X-pre)",
            s
        )));
    }

    let parse_component = |p: &str| {
        p.parse::<u64>().map_err(|_| {
            ConfigError::ValidationError(format!("'{}' contains non-numeric component '{}'", s, p))
        })
    };

    Ok(Version::Semver(
        parse_component(parts[0])?,
        parse_component(parts[1])?,
        parse_component(parts[2])?,
        pre,
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

/// Result of comparing CLI and server versions.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CompatibilityStatus {
    /// Versions are fully compatible.
    Compatible,
    /// Versions differ only in minor/patch, still compatible.
    MinorDrift {
        /// Whether the CLI version is newer than the server version.
        cli_newer: bool,
    },
    /// Major version mismatch, may be incompatible.
    MajorMismatch {
        /// Whether the CLI version is newer than the server version.
        cli_newer: bool,
    },
}

/// Compares CLI and server versions.
pub fn check_compatibility(cli: &Version, server: &Version) -> CompatibilityStatus {
    match (cli, server) {
        (Version::Latest, _) | (_, Version::Latest) => CompatibilityStatus::Compatible,
        (
            Version::Semver(cli_maj, cli_min, cli_patch, cli_pre),
            Version::Semver(srv_maj, srv_min, srv_patch, srv_pre),
        ) => {
            if (cli_maj, cli_min, cli_patch, cli_pre) == (srv_maj, srv_min, srv_patch, srv_pre) {
                return CompatibilityStatus::Compatible;
            }
            if cli_maj != srv_maj {
                return CompatibilityStatus::MajorMismatch {
                    cli_newer: (cli_maj, cli_min, cli_patch) > (srv_maj, srv_min, srv_patch),
                };
            }
            CompatibilityStatus::MinorDrift {
                cli_newer: (cli_min, cli_patch) > (srv_min, srv_patch),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extract_tag_parses_correctly() {
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
        assert_eq!(parse_image_version("img:latest").unwrap(), Version::Latest);
        assert_eq!(parse_image_version("img").unwrap(), Version::Latest);
        assert_eq!(
            parse_image_version("img:v1.2.3").unwrap(),
            Version::Semver(1, 2, 3, None)
        );
        assert_eq!(
            parse_image_version("img:v1.2.3-beta").unwrap(),
            Version::Semver(1, 2, 3, Some("beta".into()))
        );
        assert_eq!(
            parse_image_version(
                "img:v1.2.3@sha256:43be85a382dd4f2cf7fb52db998730f9d6da8391ba05d179ba1cabca3a8c2b20"
            )
            .unwrap(),
            Version::Semver(1, 2, 3, None)
        );
        assert!(parse_image_version("img:1.2.3").is_err());
        assert!(parse_image_version("img:v1.2").is_err());
        assert!(parse_image_version("img:v1.2.x").is_err());
    }

    #[test]
    fn parse_pkg_version_variants() {
        assert_eq!(
            parse_pkg_version("0.1.1").unwrap(),
            Version::Semver(0, 1, 1, None)
        );
        assert_eq!(
            parse_pkg_version("0.1.1-beta").unwrap(),
            Version::Semver(0, 1, 1, Some("beta".into()))
        );
        assert_eq!(
            parse_pkg_version("1.0.0-rc1").unwrap(),
            Version::Semver(1, 0, 0, Some("rc1".into()))
        );
        assert!(parse_pkg_version("1.2").is_err());
        assert!(parse_pkg_version("1.2.x").is_err());
    }

    #[test]
    fn downgrade_detection() {
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

    #[test]
    fn pre_release_is_older_than_release() {
        let pre = Version::Semver(1, 0, 0, Some("beta".into()));
        let rel = Version::Semver(1, 0, 0, None);
        assert!(pre.is_downgrade_from(&rel));
        assert!(!rel.is_downgrade_from(&pre));
        assert!(!pre.is_downgrade_from(&pre));
    }

    #[test]
    fn check_compatibility_cases() {
        let v = |s: &str| parse_pkg_version(s).unwrap();

        assert_eq!(
            check_compatibility(&v("0.1.1"), &v("0.1.1")),
            CompatibilityStatus::Compatible
        );
        assert_eq!(
            check_compatibility(&v("0.1.1-beta"), &v("0.1.1-beta")),
            CompatibilityStatus::Compatible
        );
        assert_eq!(
            check_compatibility(&v("0.2.0"), &v("0.1.5")),
            CompatibilityStatus::MinorDrift { cli_newer: true }
        );
        assert_eq!(
            check_compatibility(&v("0.1.0"), &v("0.2.0")),
            CompatibilityStatus::MinorDrift { cli_newer: false }
        );
        assert_eq!(
            check_compatibility(&v("1.0.0"), &v("0.9.0")),
            CompatibilityStatus::MajorMismatch { cli_newer: true }
        );
        assert_eq!(
            check_compatibility(&v("0.9.0"), &v("1.0.0")),
            CompatibilityStatus::MajorMismatch { cli_newer: false }
        );
        // Latest is always compatible.
        assert_eq!(
            check_compatibility(&Version::Latest, &v("1.0.0")),
            CompatibilityStatus::Compatible
        );
        assert_eq!(
            check_compatibility(&v("1.0.0"), &Version::Latest),
            CompatibilityStatus::Compatible
        );
    }

    #[test]
    fn display_roundtrip() {
        assert_eq!(Version::Semver(0, 1, 1, None).to_string(), "0.1.1");
        assert_eq!(
            Version::Semver(0, 1, 1, Some("beta".into())).to_string(),
            "0.1.1-beta"
        );
        assert_eq!(Version::Latest.to_string(), "latest");
    }
}
