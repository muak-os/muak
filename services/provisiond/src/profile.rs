//! Booted profile discovery.

use std::path::Path;

use anyhow::{Context as _, Result, bail};
use wizard::domain::profile::Profile;

/// Booted profile locations in precedence order.
const PROFILE_SOURCES: [&str; 2] = ["/run/state/profile.toml", "/run/boot/profile.toml"];

/// Loads the booted profile.
///
/// # Errors
///
/// Returns an error when no source carries a profile or parsing fails.
pub(crate) fn load() -> Result<Profile> {
    load_with_bytes().map(|(profile, _)| profile)
}

/// Loads the booted profile together with the exact TOML bytes backing it.
///
/// # Errors
///
/// Returns an error when no source carries a profile or parsing fails.
pub(crate) fn load_with_bytes() -> Result<(Profile, Vec<u8>)> {
    let paths: Vec<&Path> = PROFILE_SOURCES.iter().map(Path::new).collect();

    load_with_bytes_from(&paths)
}

fn load_with_bytes_from(paths: &[&Path]) -> Result<(Profile, Vec<u8>)> {
    for path in paths {
        if let Ok(bytes) = std::fs::read(path) {
            let profile = Profile::from_toml(&bytes)
                .with_context(|| format!("invalid booted profile {}", path.display()))?;
            kmsg::info!("Loaded booted profile from {}", path.display());

            return Ok((profile, bytes));
        }
    }

    bail!(
        "no booted profile found. The boot image must embed \
         a profile.toml or the system must have one recorded on STATE"
    )
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
        let profile = Profile::new(None, customization, kernel);
        let id = profile.profile_id().expect("id");

        // ASSERT
        assert_eq!(id.to_string().len(), 64);
    }

    #[test]
    fn first_existing_source_wins() {
        // ARRANGE
        let dir = tempdir().expect("tempdir");
        let first = dir.path().join("first.toml");
        let second = dir.path().join("second.toml");
        std::fs::write(&first, VALID).expect("write first");
        std::fs::write(&second, VALID).expect("write second");

        // ACT
        let (profile, bytes) = load_with_bytes_from(&[&first, &second]).expect("load");

        // ASSERT
        assert_eq!(bytes, VALID);
        assert_eq!(profile.kernel().source(), "muak-os/linux");
    }

    #[test]
    fn later_source_used_when_earlier_missing() {
        // ARRANGE
        let dir = tempdir().expect("tempdir");
        let missing = dir.path().join("missing.toml");
        let second = dir.path().join("second.toml");
        std::fs::write(&second, VALID).expect("write second");

        // ACT
        let (profile, bytes) = load_with_bytes_from(&[&missing, &second]).expect("load");

        // ASSERT
        assert_eq!(bytes, VALID);
        assert_eq!(profile.kernel().source(), "muak-os/linux");
    }

    #[test]
    fn missing_everywhere_is_a_hard_error() {
        // ARRANGE
        let dir = tempdir().expect("tempdir");
        let missing = dir.path().join("missing.toml");

        // ACT
        let result = load_with_bytes_from(&[&missing]);

        // ASSERT
        let error = result.expect_err("missing profile must be a hard error");
        let message = error.to_string();
        assert!(message.contains("no booted profile found"), "{message}");
    }

    #[test]
    fn invalid_profile_content_is_an_error() {
        // ARRANGE
        let dir = tempdir().expect("tempdir");
        let broken = dir.path().join("broken.toml");
        std::fs::write(&broken, b"not a profile").expect("write broken");

        // ACT
        let result = load_with_bytes_from(&[&broken]);

        // ASSERT
        assert!(result.is_err(), "invalid profile content must be an error");
    }
}
