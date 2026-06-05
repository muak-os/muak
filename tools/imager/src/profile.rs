//! Version-neutral, content-addressed customization document.

use core::fmt;

use ring::digest;
use serde::{Deserialize, Serialize};

use crate::error::{ImagerError, Result};

/// Top-level profile document used as input for the imager.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Profile {
    overlay: Option<OverlaySpec>,
    customization: CustomizationSpec,
}

impl Profile {
    /// Deserializes and validates a profile from TOML bytes.
    ///
    /// # Errors
    ///
    /// Returns an error when the bytes are not UTF-8, TOML parsing fails, or
    /// semantic validation fails.
    pub fn from_toml(bytes: &[u8]) -> Result<Self> {
        let spec: Self = toml::from_str(core::str::from_utf8(bytes).map_err(|_error| {
            ImagerError::ProfileValidation("profile is not valid UTF-8".into())
        })?)
        .map_err(|e| {
            ImagerError::ProfileValidation(format!("failed to parse profile TOML: {e}"))
        })?;

        Ok(spec)
    }

    /// Creates a profile from pre-validated components.
    #[must_use]
    pub fn new(overlay: Option<OverlaySpec>, customization: CustomizationSpec) -> Self {
        Self {
            overlay,
            customization,
        }
    }

    /// Returns the optional overlay spec.
    #[must_use]
    pub fn overlay(&self) -> Option<&OverlaySpec> {
        self.overlay.as_ref()
    }

    /// Returns the customization spec.
    #[must_use]
    pub fn customization(&self) -> &CustomizationSpec {
        &self.customization
    }

    /// Serializes to canonical TOML bytes with extensions sorted for a stable hash.
    ///
    /// # Errors
    ///
    /// Returns an error when TOML serialization fails.
    pub fn canonical_bytes(&self) -> Result<Vec<u8>> {
        let mut normalized = self.clone();
        normalized.customization.extensions.sort();

        Ok(toml::to_string(&normalized)
            .map_err(|e| {
                ImagerError::ProfileValidation(format!("failed to serialize profile to TOML: {e}"))
            })?
            .into_bytes())
    }

    /// Computes the stable SHA-256 content address over canonical bytes.
    ///
    /// # Errors
    ///
    /// Returns an error when canonical serialization fails.
    pub fn id(&self) -> Result<Id> {
        let bytes = self.canonical_bytes()?;
        let digest = digest::digest(&digest::SHA256, &bytes);

        Ok(Id(hex_encode(digest.as_ref())))
    }
}

/// Stable content-addressed profile identifier.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Id(String);

impl Id {
    /// Returns the profile ID as a string slice.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for Id {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// Optional overlay that selects a board-specific boot asset set from an OCI image.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OverlaySpec {
    #[serde(deserialize_with = "non_empty")]
    name: String,
    #[serde(deserialize_with = "non_empty")]
    image: String,
}

impl OverlaySpec {
    /// Creates a validated overlay spec.
    ///
    /// # Errors
    ///
    /// Returns an error when `name` or `image` is empty.
    pub fn new(name: String, image: String) -> Result<Self> {
        if name.is_empty() {
            return Err(ImagerError::ProfileValidation(
                "overlay.name must not be empty".into(),
            ));
        }
        if image.is_empty() {
            return Err(ImagerError::ProfileValidation(
                "overlay.image must not be empty".into(),
            ));
        }
        Ok(Self { name, image })
    }

    /// Returns the overlay name.
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Returns the overlay image reference.
    #[must_use]
    pub fn image(&self) -> &str {
        &self.image
    }
}

/// User-supplied customization, version-neutral and platform-neutral.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CustomizationSpec {
    #[serde(default, deserialize_with = "non_empty_vec")]
    extensions: Vec<String>,
}

impl CustomizationSpec {
    /// Creates a validated customization spec.
    ///
    /// # Errors
    ///
    /// Returns an error when any extension name is empty.
    pub fn new(extensions: Vec<String>) -> Result<Self> {
        if extensions.iter().any(String::is_empty) {
            return Err(ImagerError::ProfileValidation(
                "extension name must not be empty".into(),
            ));
        }
        Ok(Self { extensions })
    }

    /// Returns the selected extensions.
    #[must_use]
    pub fn extensions(&self) -> &[String] {
        &self.extensions
    }
}

/// Rejects empty strings during deserialization.
fn non_empty<'de, D: serde::Deserializer<'de>>(deserializer: D) -> Result<String, D::Error> {
    use serde::de::Error;
    let value = String::deserialize(deserializer)?;
    if value.is_empty() {
        return Err(Error::custom("must not be empty"));
    }

    Ok(value)
}

/// Rejects vectors containing empty strings during deserialization.
fn non_empty_vec<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<Vec<String>, D::Error> {
    use serde::de::Error;
    let vec = Vec::<String>::deserialize(deserializer)?;
    if vec.iter().any(String::is_empty) {
        return Err(Error::custom("extension name must not be empty"));
    }

    Ok(vec)
}

fn hex_encode(bytes: &[u8]) -> String {
    const HEX: &[u8] = b"0123456789abcdef";
    let mut hex = String::with_capacity(bytes.len().saturating_mul(2));
    for byte in bytes {
        let hi = HEX.get(usize::from(byte >> 4)).copied().unwrap_or(b'0');
        let lo = HEX.get(usize::from(byte & 0x0f)).copied().unwrap_or(b'0');
        hex.push(char::from(hi));
        hex.push(char::from(lo));
    }

    hex
}

#[cfg(test)]
mod tests {
    use super::*;

    fn minimal_toml() -> &'static str {
        r#"
[customization]
extensions = ["muak-os/qemu"]
"#
    }

    fn overlay_toml() -> &'static str {
        r#"
[overlay]
name = "rpi_generic"
image = "muak-os/sbc-raspberrypi"

[customization]
extensions = ["muak-os/qemu"]
"#
    }

    #[test]
    fn parses_minimal_profile() {
        // ARRANGE / ACT
        let spec = Profile::from_toml(minimal_toml().as_bytes()).expect("parse");

        // ASSERT
        assert!(spec.overlay().is_none());
        assert_eq!(spec.customization().extensions(), vec!["muak-os/qemu"]);
    }

    #[test]
    fn parses_overlay_profile() {
        // ARRANGE / ACT
        let spec = Profile::from_toml(overlay_toml().as_bytes()).expect("parse");

        // ASSERT
        let ov = spec.overlay().expect("overlay present");
        assert_eq!(ov.name(), "rpi_generic");
        assert_eq!(ov.image(), "muak-os/sbc-raspberrypi");
    }

    #[test]
    fn id_is_stable() {
        // ARRANGE
        let spec = Profile::from_toml(minimal_toml().as_bytes()).expect("parse");

        // ACT
        let id1 = spec.id().expect("id");
        let id2 = spec.id().expect("id");

        // ASSERT
        assert_eq!(id1, id2);
        assert_eq!(id1.as_str().len(), 64);
    }

    #[test]
    fn extension_order_does_not_affect_id() {
        // ARRANGE
        let first =
            Profile::from_toml(b"[customization]\nextensions = [\"muak-os/a\", \"muak-os/b\"]")
                .expect("parse");
        let second =
            Profile::from_toml(b"[customization]\nextensions = [\"muak-os/b\", \"muak-os/a\"]")
                .expect("parse");

        // ACT / ASSERT
        assert_eq!(first.id().expect("id"), second.id().expect("id"));
    }

    #[test]
    fn rejects_unknown_fields() {
        // ARRANGE
        let raw = b"unknown_key = true\n[customization]\nextensions = []";

        // ACT
        let err = Profile::from_toml(raw).expect_err("should fail");

        // ASSERT
        assert!(matches!(err, ImagerError::ProfileValidation(_)));
    }

    #[test]
    fn rejects_overlay_with_empty_name() {
        // ARRANGE
        let raw =
            b"[overlay]\nname = \"\"\nimage = \"muak-os/sbc-raspberrypi\"\n[customization]\nextensions = []";

        // ACT
        let err = Profile::from_toml(raw).expect_err("should fail");

        // ASSERT
        assert!(matches!(err, ImagerError::ProfileValidation(_)));
    }

    #[test]
    fn rejects_overlay_with_empty_image() {
        // ARRANGE
        let raw = b"[overlay]\nname = \"rpi\"\nimage = \"\"\n[customization]\nextensions = []";

        // ACT
        let err = Profile::from_toml(raw).expect_err("should fail");

        // ASSERT
        assert!(matches!(err, ImagerError::ProfileValidation(_)));
    }

    #[test]
    fn rejects_empty_extension_name() {
        // ARRANGE
        let raw = b"[customization]\nextensions = [\"\"]";

        // ACT
        let err = Profile::from_toml(raw).expect_err("should fail");

        // ASSERT
        assert!(matches!(err, ImagerError::ProfileValidation(_)));
    }

    #[test]
    fn parses_empty_extensions() {
        // ARRANGE / ACT
        let spec = Profile::from_toml(b"[customization]\nextensions = []").expect("parse");

        // ASSERT
        assert!(spec.customization().extensions().is_empty());
    }

    #[test]
    fn overlay_spec_new_rejects_empty_name() {
        // ARRANGE / ACT
        let err = OverlaySpec::new(String::new(), "image".into()).expect_err("should fail");

        // ASSERT
        assert!(matches!(err, ImagerError::ProfileValidation(_)));
    }

    #[test]
    fn overlay_spec_new_rejects_empty_image() {
        // ARRANGE / ACT
        let err = OverlaySpec::new("name".into(), String::new()).expect_err("should fail");

        // ASSERT
        assert!(matches!(err, ImagerError::ProfileValidation(_)));
    }

    #[test]
    fn customization_spec_new_rejects_empty_extension() {
        // ARRANGE / ACT
        let err = CustomizationSpec::new(vec![String::new()]).expect_err("should fail");

        // ASSERT
        assert!(matches!(err, ImagerError::ProfileValidation(_)));
    }

    #[test]
    fn profile_new_accepts_valid_overlay() {
        // ARRANGE
        let overlay = OverlaySpec::new("name".into(), "image".into()).expect("valid overlay");
        let customization = CustomizationSpec::new(vec![]).expect("valid customization");

        // ACT
        let profile = Profile::new(Some(overlay), customization);
        let id = profile.id().expect("id");

        // ASSERT
        assert_eq!(id.as_str().len(), 64);
    }

    #[test]
    fn profile_new_accepts_no_overlay() {
        // ARRANGE
        let customization = CustomizationSpec::new(vec![]).expect("valid customization");

        // ACT
        let profile = Profile::new(None, customization);
        let id = profile.id().expect("id");

        // ASSERT
        assert_eq!(id.as_str().len(), 64);
    }
}
