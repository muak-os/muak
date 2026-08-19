//! Booted profile discovery.

use std::path::Path;

use anyhow::{Context as _, Result};
use wizard::domain::profile::{CustomizationSpec, KernelSpec, Profile};
use wizard::domain::release;

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
        let kernel =
            KernelSpec::new(release::manifest()?.kernel().source().to_owned()).context("kernel")?;

        Ok(Profile::new(None, customization, kernel))
    }
}

#[cfg(test)]
mod tests {
    use wizard::domain::profile::{CustomizationSpec, KernelSpec, Profile};

    #[test]
    fn load_parses_minimal_profile() {
        // ARRANGE / ACT
        let parsed = Profile::from_toml(
            b"[kernel]\nsource = \"muak-os/kernel\"\n[customization]\nextensions = []",
        )
        .expect("parse");

        // ASSERT
        assert!(parsed.overlay().is_none());
        assert_eq!(parsed.kernel().source(), "muak-os/kernel");
        assert!(parsed.customization().extensions().is_empty());
    }

    #[test]
    fn empty_profile_is_valid() {
        // ARRANGE
        let customization = CustomizationSpec::new(vec![]).expect("empty customization");
        let kernel = KernelSpec::new("muak-os/kernel".to_owned()).expect("kernel");

        // ACT
        let profile = Profile::new(None, customization, kernel);
        let id = profile.profile_id().expect("id");

        // ASSERT
        assert_eq!(id.to_string().len(), 64);
    }
}
