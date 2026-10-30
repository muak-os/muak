//! Release version from the booted image metadata.

use std::fs;
use std::path::Path;

use super::PollState;

const BOOT_VERSION_PATH: &str = "/run/boot/version";

pub(super) fn read(poll: &mut PollState) -> String {
    if let Some(version) = poll.version_cache.as_ref() {
        return version.clone();
    }

    let version = version_from(Path::new(BOOT_VERSION_PATH)).unwrap_or_default();
    poll.version_cache = Some(version.clone());

    version
}

fn version_from(path: &Path) -> Option<String> {
    let contents = fs::read_to_string(path).ok()?;
    let version = contents.trim();

    (!version.is_empty()).then(|| version.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_trimmed_boot_version() {
        // ARRANGE
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("version");
        std::fs::write(&path, "v1.0.0-nightly\n").unwrap();

        // ACT
        let version = version_from(&path);

        // ASSERT
        assert_eq!(version.as_deref(), Some("v1.0.0-nightly"));
    }

    #[test]
    fn empty_boot_version_is_ignored() {
        // ARRANGE
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("version");
        std::fs::write(&path, "  \n").unwrap();

        // ACT
        let version = version_from(&path);

        // ASSERT
        assert_eq!(version, None);
    }

    #[test]
    fn missing_boot_version_is_ignored() {
        // ARRANGE
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("version");

        // ACT
        let version = version_from(&path);

        // ASSERT
        assert_eq!(version, None);
    }
}
