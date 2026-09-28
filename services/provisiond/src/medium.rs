//! Discovery of the booted medium's metadata.

use std::path::Path;

use anyhow::{Context as _, Result};
use wizard::domain::profile::Profile;

const PROFILE_PATH: &str = "/run/boot/profile.toml";
const VERSION_PATH: &str = "/run/boot/version";

/// Loads the booted medium's profile.
///
/// # Errors
///
/// Returns an error when the boot image carries no profile or parsing fails.
pub(crate) fn profile() -> Result<Profile> {
    profile_from(Path::new(PROFILE_PATH))
}

/// Loads the booted medium's release version.
///
/// # Errors
///
/// Returns an error when the boot image carries no version metadata.
pub(crate) fn version() -> Result<String> {
    version_from(Path::new(VERSION_PATH))
}

fn profile_from(path: &Path) -> Result<Profile> {
    let bytes = std::fs::read(path)
        .with_context(|| format!("failed to read the booted profile {}", path.display()))?;
    let profile = Profile::from_toml(&bytes)
        .with_context(|| format!("invalid booted profile {}", path.display()))?;
    kmsg::info!("Loaded booted profile from {}", path.display());

    Ok(profile)
}

fn version_from(path: &Path) -> Result<String> {
    let raw = std::fs::read_to_string(path)
        .with_context(|| format!("failed to read the booted version {}", path.display()))?;
    let version = raw.trim();
    if version.is_empty() {
        return Err(anyhow::anyhow!(
            "booted version {} is empty",
            path.display()
        ));
    }

    Ok(version.to_owned())
}

#[cfg(test)]
mod tests {
    use tempfile::tempdir;
    use wizard::domain::profile::{CustomizationSpec, KernelSpec, Profile};

    use super::*;

    const VALID: &[u8] = b"[kernel]\nsource = \"muak-os/linux\"\n[customization]\nextensions = []";

    #[test]
    fn load_parses_minimal_profile() {
        // ARRANGE / ACT
        let parsed = Profile::from_toml(VALID).expect("parse");

        // ASSERT
        assert!(parsed.overlay().is_none());
        assert_eq!(parsed.kernel().source(), "muak-os/linux");
        assert!(parsed.customization().extensions().is_empty());
    }

    #[test]
    fn empty_profile_is_valid() {
        // ARRANGE
        let customization = CustomizationSpec::new(vec![]).expect("empty customization");
        let kernel = KernelSpec::new("muak-os/linux".to_owned()).expect("kernel");

        // ACT
        let doc = Profile::new(None, customization, kernel);
        let id = doc.profile_id().expect("id");

        // ASSERT
        assert_eq!(id.to_string().len(), 64);
    }

    #[test]
    fn loads_profile_from_the_boot_image_path() {
        // ARRANGE
        let dir = tempdir().expect("tempdir");
        let path = dir.path().join("profile.toml");
        std::fs::write(&path, VALID).expect("write profile");

        // ACT
        let profile = profile_from(&path).expect("load");

        // ASSERT
        assert_eq!(profile.kernel().source(), "muak-os/linux");
    }

    #[test]
    fn missing_profile_is_a_hard_error() {
        // ARRANGE
        let dir = tempdir().expect("tempdir");
        let missing = dir.path().join("profile.toml");

        // ACT
        let result = profile_from(&missing);

        // ASSERT
        let error = result.expect_err("missing profile must be an error");
        let message = error.to_string();
        assert!(message.contains("booted profile"), "{message}");
    }

    #[test]
    fn invalid_profile_content_is_an_error() {
        // ARRANGE
        let dir = tempdir().expect("tempdir");
        let broken = dir.path().join("profile.toml");
        std::fs::write(&broken, b"not a profile").expect("write broken");

        // ACT
        let result = profile_from(&broken);

        // ASSERT
        assert!(result.is_err(), "invalid profile content must be an error");
    }

    #[test]
    fn version_reads_trimmed_content() {
        // ARRANGE
        let dir = tempdir().expect("tempdir");
        let path = dir.path().join("version");
        std::fs::write(&path, "v1.2.3\n").expect("write version");

        // ACT
        let version = version_from(&path).expect("version");

        // ASSERT
        assert_eq!(version, "v1.2.3");
    }

    #[test]
    fn missing_version_is_a_hard_error() {
        // ARRANGE
        let dir = tempdir().expect("tempdir");
        let missing = dir.path().join("version");

        // ACT
        let result = version_from(&missing);

        // ASSERT
        let error = result.expect_err("missing version must be an error");
        let message = error.to_string();
        assert!(message.contains("booted version"), "{message}");
    }
}
