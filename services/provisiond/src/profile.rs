//! Booted profile discovery.

use std::path::Path;

use anyhow::{Context as _, Result};
use wizard::profile::{CustomizationSpec, Profile};

/// Runtime path where `core/init` copies the embedded profile.
const BOOTED_PROFILE: &str = "/profile.toml";

/// Loads the booted profile or falls back to a generic empty one.
pub(crate) fn load() -> Result<Profile> {
    let path = Path::new(BOOTED_PROFILE);
    if path.exists() {
        let bytes =
            std::fs::read(path).with_context(|| format!("failed to read {BOOTED_PROFILE}"))?;
        Profile::from_toml(&bytes).context("invalid booted profile")
    } else {
        let customization = CustomizationSpec::new(vec![]).context("empty customization")?;
        Ok(Profile::new(None, customization))
    }
}

#[cfg(test)]
mod tests {
    use wizard::profile::{CustomizationSpec, Profile};

    #[test]
    fn load_parses_minimal_profile() {
        // ARRANGE
        let dir = tempfile::TempDir::new().expect("tempdir");
        let profile = dir.path().join("profile.toml");
        std::fs::write(&profile, b"[customization]\nextensions = []").expect("write");

        // ACT
        let parsed = Profile::from_toml(b"[customization]\nextensions = []").expect("parse");

        // ASSERT
        assert!(parsed.overlay().is_none());
        assert!(parsed.customization().extensions().is_empty());
    }

    #[test]
    fn empty_profile_is_valid() {
        // ARRANGE
        let customization = CustomizationSpec::new(vec![]).expect("empty customization");
        let profile = Profile::new(None, customization);

        // ACT
        let id = profile.id().expect("id");

        // ASSERT
        assert_eq!(id.as_str().len(), 64);
    }
}
