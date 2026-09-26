//! Auto-selection policy: bump repositories to their newest version tag.

use koci::registry;

use crate::error::{KataError, Result};

/// Newest version tag of `repository`, for entries without an explicit selection.
///
/// # Errors
///
/// Returns an error when the registry listing fails or the repository has no version tag.
pub(crate) fn newest_tag(registry: &str, repository: &str) -> Result<String> {
    let tags = registry::tags(&format!("{registry}/{repository}"))
        .map_err(|error| KataError::Registry(error.to_string()))?;

    newest(&tags)
}

/// Pick the newest tag by semver precedence: pre-release suffixes (`-beta`, `-rc1`, ...)
/// rank below their release, build metadata is ignored, and tags without a
/// `vMAJOR.MINOR.PATCH` version are ignored.
///
/// # Errors
///
/// Returns an error when no tag carries a version.
pub(crate) fn newest(tags: &[String]) -> Result<String> {
    tags.iter()
        .filter_map(|tag| {
            let version = semver::Version::parse(tag.strip_prefix('v')?).ok()?;
            Some((version, tag))
        })
        .max()
        .map(|(_, tag)| tag.clone())
        .ok_or_else(|| KataError::Registry("no version tag found to select".to_owned()))
}

#[cfg(test)]
mod tests {
    use super::newest;
    use crate::error::KataError;

    fn tags(values: &[&str]) -> Vec<String> {
        values.iter().map(ToString::to_string).collect()
    }

    #[test]
    fn newest_picks_the_highest_version() {
        // ARRANGE
        let tags = tags(&["v1.9.9", "v1.10.0", "v1.2.3", "latest", "main"]);

        // ACT / ASSERT
        assert_eq!(newest(&tags).expect("a version tag"), "v1.10.0");
    }

    #[test]
    fn newest_ranks_prereleases_below_their_release() {
        // ARRANGE
        let tags = tags(&["v1.0.0-beta", "v1.0.0-alpha", "v1.0.0", "v1.0.0-rc1"]);

        // ACT / ASSERT
        assert_eq!(newest(&tags).expect("a version tag"), "v1.0.0");
    }

    #[test]
    fn newest_orders_prerelease_keywords_and_numbers() {
        // ARRANGE
        let tags = tags(&["v1.0.0-alpha", "v1.0.0-beta", "v1.0.0-rc.2", "v1.0.0-rc.10"]);

        // ACT / ASSERT
        assert_eq!(newest(&tags).expect("a version tag"), "v1.0.0-rc.10");
    }

    #[test]
    fn newest_ignores_malformed_versions() {
        // ARRANGE
        let tags = tags(&["v1.2", "x1.2.3", "v1.2.3.4", "v2.0.0"]);

        // ACT / ASSERT
        assert_eq!(newest(&tags).expect("a version tag"), "v2.0.0");
    }

    #[test]
    fn newest_fails_without_any_version_tag() {
        // ARRANGE
        let tags = tags(&["latest", "main", "v1.2"]);

        // ACT / ASSERT
        let error = newest(&tags).expect_err("no version tag must fail");
        assert!(matches!(error, KataError::Registry(_)));
    }
}
