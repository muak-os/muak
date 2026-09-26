//! Catalog release parsing and comparison.

use semver::Version;

use crate::error::{ConfigError, Result};

/// Parses a catalog release (`vMAJOR.MINOR.PATCH[-PRERELEASE]`).
///
/// # Errors
///
/// Returns an error when `release` is not a valid semantic version.
pub fn parse_release(release: &str) -> Result<Version> {
    let core = release.strip_prefix('v').unwrap_or(release);

    Version::parse(core).map_err(|error| {
        ConfigError::ValidationError(format!("invalid release '{release}': {error}"))
    })
}

/// Validates that `new_release` is not a downgrade relative to `current_release`.
///
/// # Errors
///
/// Returns an error when `new_release` is older than `current_release`, or
/// either release is not a valid version.
pub fn check_no_downgrade(new_release: &str, current_release: &str) -> Result<()> {
    let new_version = parse_release(new_release)?;
    let current_version = parse_release(current_release)?;

    if new_version < current_version {
        return Err(ConfigError::ValidationError(format!(
            "downgrade rejected: target release '{new_release}' is older than installed release '{current_release}'"
        )));
    }

    Ok(())
}

/// Result of comparing CLI and server versions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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

/// Compares CLI and server versions; unparsable versions count as compatible.
#[must_use]
pub fn check_compatibility(cli: &str, server: &str) -> CompatibilityStatus {
    let (Ok(cli), Ok(server)) = (Version::parse(cli), Version::parse(server)) else {
        return CompatibilityStatus::Compatible;
    };

    if cli == server {
        return CompatibilityStatus::Compatible;
    }

    if cli.major != server.major {
        CompatibilityStatus::MajorMismatch {
            cli_newer: cli > server,
        }
    } else {
        CompatibilityStatus::MinorDrift {
            cli_newer: cli > server,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{CompatibilityStatus, check_compatibility, check_no_downgrade, parse_release};

    #[test]
    fn parse_release_accepts_pinned_and_plain_versions() {
        // ACT / ASSERT
        assert_eq!(
            parse_release("v1.2.3").map(|v| v.to_string()).ok(),
            Some("1.2.3".to_owned())
        );
        assert_eq!(
            parse_release("1.2.3").map(|v| v.to_string()).ok(),
            Some("1.2.3".to_owned())
        );
        assert!(parse_release("v1.2.3-beta").is_ok());
        assert!(parse_release("v1.2.3-rc.10+build").is_ok());
    }

    #[test]
    fn parse_release_rejects_malformed_releases() {
        // ACT / ASSERT
        assert!(parse_release("").is_err());
        assert!(parse_release("v1.2").is_err());
        assert!(parse_release("v1.2.x").is_err());
        assert!(parse_release("latest").is_err());
        assert!(parse_release("stable").is_err());
    }

    #[test]
    fn downgrade_detection() {
        // ACT / ASSERT
        assert!(check_no_downgrade("v0.1.0", "v0.2.0").is_err());
        assert!(check_no_downgrade("v1.0.0", "v2.0.0").is_err());
        assert!(check_no_downgrade("v1.2.3", "v1.2.4").is_err());

        assert!(check_no_downgrade("v1.2.3", "v1.2.3").is_ok());
        assert!(check_no_downgrade("v1.3.0", "v1.2.9").is_ok());
        assert!(check_no_downgrade("v2.0.0", "v1.99.99").is_ok());
    }

    #[test]
    fn pre_release_is_older_than_its_release() {
        // ACT / ASSERT
        assert!(check_no_downgrade("v1.0.0-beta", "v1.0.0").is_err());
        assert!(check_no_downgrade("v1.0.0", "v1.0.0-rc1").is_ok());
        assert!(check_no_downgrade("v1.2.0-beta", "v1.1.0").is_ok());
    }

    #[test]
    fn check_compatibility_cases() {
        // ACT / ASSERT
        assert_eq!(
            check_compatibility("0.1.1", "0.1.1"),
            CompatibilityStatus::Compatible
        );
        assert_eq!(
            check_compatibility("0.1.1-beta", "0.1.1-beta"),
            CompatibilityStatus::Compatible
        );
        assert_eq!(
            check_compatibility("0.2.0", "0.1.5"),
            CompatibilityStatus::MinorDrift { cli_newer: true }
        );
        assert_eq!(
            check_compatibility("0.1.0", "0.2.0"),
            CompatibilityStatus::MinorDrift { cli_newer: false }
        );
        assert_eq!(
            check_compatibility("1.0.0", "0.9.0"),
            CompatibilityStatus::MajorMismatch { cli_newer: true }
        );
        assert_eq!(
            check_compatibility("0.9.0", "1.0.0"),
            CompatibilityStatus::MajorMismatch { cli_newer: false }
        );
    }

    #[test]
    fn unparsable_versions_count_as_compatible() {
        // ACT / ASSERT
        assert_eq!(
            check_compatibility("latest", "1.0.0"),
            CompatibilityStatus::Compatible
        );
        assert_eq!(
            check_compatibility("1.0.0", "stable"),
            CompatibilityStatus::Compatible
        );
    }
}
