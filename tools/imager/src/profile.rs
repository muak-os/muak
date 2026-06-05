//! Version-neutral, content-addressed customization document.

use core::fmt;

use ring::digest;
use serde::{Deserialize, Serialize};

use crate::error::{ImagerError, Result};

/// Optional overlay that selects a board-specific boot asset set from an OCI image.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OverlaySpec {
    /// Overlay name (e.g. board variant).
    pub name: String,
    /// OCI image reference containing the overlay files.
    pub image: String,
}

/// User-supplied customization, version-neutral and platform-neutral.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CustomizationSpec {
    /// List of OCI extension references to include.
    #[serde(default)]
    pub extensions: Vec<String>,
}

/// Top-level profile document used as input for the imager.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Profile {
    /// Optional board-specific overlay.
    pub overlay: Option<OverlaySpec>,
    /// Extension and customization settings.
    pub customization: CustomizationSpec,
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
        spec.validate()?;

        Ok(spec)
    }

    /// Validates semantic rules that serde cannot enforce.
    ///
    /// # Errors
    ///
    /// Returns an error when an overlay field or extension name is empty.
    pub fn validate(&self) -> Result<()> {
        if let Some(ref ov) = self.overlay {
            validate_nonempty(&ov.name, "overlay.name")?;
            validate_nonempty(&ov.image, "overlay.image")?;
        }
        for ext in &self.customization.extensions {
            validate_nonempty(ext, "extension name")?;
        }

        Ok(())
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

/// Returns an error when `value` is empty, naming the field in the message.
fn validate_nonempty(value: &str, field: &str) -> Result<()> {
    if value.is_empty() {
        return Err(ImagerError::ProfileValidation(format!(
            "{field} must not be empty"
        )));
    }
    Ok(())
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
        assert!(spec.overlay.is_none());
        assert_eq!(spec.customization.extensions, vec!["muak-os/qemu"]);
    }

    #[test]
    fn parses_overlay_profile() {
        // ARRANGE / ACT
        let spec = Profile::from_toml(overlay_toml().as_bytes()).expect("parse");

        // ASSERT
        let ov = spec.overlay.expect("overlay present");
        assert_eq!(ov.name, "rpi_generic");
        assert_eq!(ov.image, "muak-os/sbc-raspberrypi");
    }

    #[test]
    fn profile_id_is_stable() {
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
    fn extension_order_does_not_affect_profile_id() {
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
        assert!(spec.customization.extensions.is_empty());
    }
}
