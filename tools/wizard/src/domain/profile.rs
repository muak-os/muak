//! The version-neutral, content-addressed customization spec.

use serde::{Deserialize, Serialize};

use crate::domain::identity::ProfileId;
use crate::domain::{canonical_toml, non_empty, non_empty_vec, reject_empty};
use crate::error::{Result, WizardError};

/// Version-neutral, content-addressed customization document.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Profile {
    overlay: Option<OverlaySpec>,
    customization: CustomizationSpec,
    kernel: KernelSpec,
}

impl Profile {
    /// Creates a profile from pre-validated components.
    #[must_use]
    pub const fn new(
        overlay: Option<OverlaySpec>,
        customization: CustomizationSpec,
        kernel: KernelSpec,
    ) -> Self {
        Self {
            overlay,
            customization,
            kernel,
        }
    }

    /// Deserializes and validates a profile from TOML bytes.
    ///
    /// # Errors
    ///
    /// Returns an error when the bytes are not UTF-8, TOML parsing fails, or
    /// semantic validation fails.
    pub fn from_toml(bytes: &[u8]) -> Result<Self> {
        toml::from_str(core::str::from_utf8(bytes).map_err(|_error| {
            WizardError::ProfileValidation("profile is not valid UTF-8".into())
        })?)
        .map_err(|e| WizardError::ProfileValidation(format!("failed to parse profile TOML: {e}")))
    }

    /// Returns the optional overlay spec.
    #[must_use]
    pub const fn overlay(&self) -> Option<&OverlaySpec> {
        self.overlay.as_ref()
    }

    /// Returns the customization spec.
    #[must_use]
    pub const fn customization(&self) -> &CustomizationSpec {
        &self.customization
    }

    /// Returns the kernel spec.
    #[must_use]
    pub const fn kernel(&self) -> &KernelSpec {
        &self.kernel
    }

    /// Computes the stable profile identity.
    ///
    /// # Errors
    ///
    /// Returns an error when canonical serialization fails.
    pub fn profile_id(&self) -> Result<ProfileId> {
        Ok(ProfileId::new(&self.canonical_bytes()?))
    }

    /// Serializes the profile with normalized extensions to canonical bytes.
    ///
    /// # Errors
    ///
    /// Returns an error when extension normalization or serialization fails.
    pub fn canonical_bytes(&self) -> Result<Vec<u8>> {
        let mut normalized = self.clone();
        normalized.customization.extensions = canonical_extensions(&self.customization.extensions)?;

        canonical_toml(&normalized)
    }
}

/// Normalizes a legacy extension alias to its canonical logical name.
#[must_use]
pub fn normalize_extension_name(name: &str) -> &str {
    match name {
        "qemu" => "muak-os/qemu",
        other => other,
    }
}

/// Sorts and normalizes extension names, rejecting duplicates.
///
/// # Errors
///
/// Returns an error when two names normalize to the same identity.
fn canonical_extensions(names: &[String]) -> Result<Vec<String>> {
    let mut names = names
        .iter()
        .map(|name| normalize_extension_name(name).to_owned())
        .collect::<Vec<_>>();
    names.sort_unstable();

    if names.windows(2).any(|pair| pair.first() == pair.get(1)) {
        return Err(WizardError::ProfileValidation(
            "duplicate extension name".to_owned(),
        ));
    }

    Ok(names)
}

/// Optional overlay that selects a board-specific boot asset set from an OCI image.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OverlaySpec {
    #[serde(deserialize_with = "non_empty")]
    name: String,
    #[serde(deserialize_with = "non_empty")]
    source: String,
}

impl OverlaySpec {
    /// Creates a validated overlay spec.
    ///
    /// # Errors
    ///
    /// Returns an error when `name` or `source` is empty.
    pub fn new(name: String, source: String) -> Result<Self> {
        reject_empty(&name, "overlay.name")?;
        reject_empty(&source, "overlay.source")?;

        Ok(Self { name, source })
    }

    /// Returns the overlay name.
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Returns the logical overlay source identity.
    #[must_use]
    pub fn source(&self) -> &str {
        &self.source
    }
}

/// Kernel package source identity, registry-neutral and version-neutral.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct KernelSpec {
    #[serde(deserialize_with = "non_empty")]
    source: String,
}

impl KernelSpec {
    /// Creates a validated kernel spec.
    ///
    /// # Errors
    ///
    /// Returns an error when `source` is empty.
    pub fn new(source: String) -> Result<Self> {
        reject_empty(&source, "kernel.source")?;

        Ok(Self { source })
    }

    /// Returns the logical kernel source identity.
    #[must_use]
    pub fn source(&self) -> &str {
        &self.source
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
            return Err(WizardError::ProfileValidation(
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

#[cfg(test)]
mod tests {
    use super::*;

    fn minimal_toml() -> &'static str {
        r#"
[kernel]
source = "muak-os/linux"

[customization]
extensions = []
"#
    }

    fn extension_toml() -> &'static str {
        r#"
[kernel]
source = "muak-os/linux"

[customization]
extensions = ["muak-os/qemu"]
"#
    }

    fn overlay_toml() -> &'static str {
        r#"
[overlay]
name = "rpi_generic"
source = "muak-os/sbc-raspberrypi"

[kernel]
source = "muak-os/linux"

[customization]
extensions = ["muak-os/qemu"]
"#
    }

    #[test]
    fn parses_minimal_profile() {
        // ARRANGE / ACT
        let doc = Profile::from_toml(minimal_toml().as_bytes()).expect("parse");

        // ASSERT
        assert!(doc.overlay().is_none());
        assert_eq!(doc.kernel().source(), "muak-os/linux");
        assert!(doc.customization().extensions().is_empty());
    }

    #[test]
    fn parses_overlay_profile() {
        // ARRANGE / ACT
        let doc = Profile::from_toml(overlay_toml().as_bytes()).expect("parse");

        // ASSERT
        let ov = doc.overlay().expect("overlay present");
        assert_eq!(ov.name(), "rpi_generic");
        assert_eq!(ov.source(), "muak-os/sbc-raspberrypi");
        assert_eq!(doc.kernel().source(), "muak-os/linux");
    }

    #[test]
    fn profile_id_is_stable() {
        // ARRANGE
        let doc = Profile::from_toml(minimal_toml().as_bytes()).expect("parse");

        // ACT
        let id1 = doc.profile_id().expect("id");
        let id2 = doc.profile_id().expect("id");

        // ASSERT
        assert_eq!(id1, id2);
        assert_eq!(id1.to_string().len(), 64);
    }

    #[test]
    fn extension_order_does_not_affect_profile_id() {
        // ARRANGE
        let first = Profile::from_toml(
            b"[kernel]\nsource = \"muak-os/linux\"\n[customization]\nextensions = [\"muak-os/a\", \"muak-os/b\"]",
        )
        .expect("parse");
        let second = Profile::from_toml(
            b"[kernel]\nsource = \"muak-os/linux\"\n[customization]\nextensions = [\"muak-os/b\", \"muak-os/a\"]",
        )
        .expect("parse");

        // ACT / ASSERT
        assert_eq!(
            first.profile_id().expect("id"),
            second.profile_id().expect("id")
        );
    }

    #[test]
    fn extension_alias_normalizes_to_same_profile_id() {
        // ARRANGE
        let aliased = Profile::from_toml(
            b"[kernel]\nsource = \"muak-os/linux\"\n[customization]\nextensions = [\"qemu\"]",
        )
        .expect("parse");
        let canonical = Profile::from_toml(extension_toml().as_bytes()).expect("parse");

        // ACT / ASSERT
        assert_eq!(
            aliased.profile_id().expect("id"),
            canonical.profile_id().expect("id")
        );
    }

    #[test]
    fn kernel_source_change_affects_profile_id() {
        // ARRANGE
        let first = Profile::from_toml(minimal_toml().as_bytes()).expect("parse");
        let second = Profile::from_toml(
            b"[kernel]\nsource = \"other/kernel\"\n[customization]\nextensions = []",
        )
        .expect("parse");

        // ACT / ASSERT
        assert_ne!(
            first.profile_id().expect("id"),
            second.profile_id().expect("id")
        );
    }

    #[test]
    fn overlay_change_affects_profile_id() {
        // ARRANGE
        let first = Profile::from_toml(overlay_toml().as_bytes()).expect("parse");
        let second = Profile::from_toml(
            b"[overlay]\nname = \"rpi_generic\"\nsource = \"other/sbc\"\n[kernel]\nsource = \"muak-os/linux\"\n[customization]\nextensions = []",
        )
        .expect("parse");

        // ACT / ASSERT
        assert_ne!(
            first.profile_id().expect("id"),
            second.profile_id().expect("id")
        );
    }

    #[test]
    fn rejects_unknown_fields() {
        // ARRANGE
        let raw = b"unknown_key = true\n[kernel]\nsource = \"muak-os/linux\"\n[customization]\nextensions = []";

        // ACT
        let err = Profile::from_toml(raw).expect_err("should fail");

        // ASSERT
        assert!(matches!(err, WizardError::ProfileValidation(_)));
    }

    #[test]
    fn rejects_overlay_with_empty_name() {
        // ARRANGE
        let raw = b"[overlay]\nname = \"\"\nsource = \"muak-os/sbc\"\n[kernel]\nsource = \"muak-os/linux\"\n[customization]\nextensions = []";

        // ACT
        let err = Profile::from_toml(raw).expect_err("should fail");

        // ASSERT
        assert!(matches!(err, WizardError::ProfileValidation(_)));
    }

    #[test]
    fn rejects_overlay_with_empty_source() {
        // ARRANGE
        let raw = b"[overlay]\nname = \"rpi\"\nsource = \"\"\n[kernel]\nsource = \"muak-os/linux\"\n[customization]\nextensions = []";

        // ACT
        let err = Profile::from_toml(raw).expect_err("should fail");

        // ASSERT
        assert!(matches!(err, WizardError::ProfileValidation(_)));
    }

    #[test]
    fn rejects_empty_extension_name() {
        // ARRANGE
        let raw = b"[kernel]\nsource = \"muak-os/linux\"\n[customization]\nextensions = [\"\"]";

        // ACT
        let err = Profile::from_toml(raw).expect_err("should fail");

        // ASSERT
        assert!(matches!(err, WizardError::ProfileValidation(_)));
    }

    #[test]
    fn rejects_duplicate_extensions() {
        // ARRANGE
        let doc = Profile::from_toml(
            b"[kernel]\nsource = \"muak-os/linux\"\n[customization]\nextensions = [\"muak-os/qemu\", \"qemu\"]",
        )
        .expect("parse");

        // ACT
        let err = doc.profile_id().expect_err("should fail");

        // ASSERT
        assert!(matches!(err, WizardError::ProfileValidation(_)));
    }

    #[test]
    fn rejects_missing_kernel() {
        // ARRANGE
        let raw = b"[customization]\nextensions = []";

        // ACT
        let err = Profile::from_toml(raw).expect_err("missing kernel must fail");

        // ASSERT
        assert!(matches!(err, WizardError::ProfileValidation(_)));
    }

    #[test]
    fn rejects_empty_kernel_source() {
        // ARRANGE
        let raw = b"[kernel]\nsource = \"\"\n[customization]\nextensions = []";

        // ACT
        let err = Profile::from_toml(raw).expect_err("empty kernel source must fail");

        // ASSERT
        assert!(matches!(err, WizardError::ProfileValidation(_)));
    }

    #[test]
    fn kernel_spec_new_rejects_empty_source() {
        // ARRANGE / ACT
        let err = KernelSpec::new(String::new()).expect_err("should fail");

        // ASSERT
        assert!(matches!(err, WizardError::ProfileValidation(_)));
    }

    #[test]
    fn overlay_spec_new_rejects_empty_name() {
        // ARRANGE / ACT
        let err = OverlaySpec::new(String::new(), "source".into()).expect_err("should fail");

        // ASSERT
        assert!(matches!(err, WizardError::ProfileValidation(_)));
    }

    #[test]
    fn overlay_spec_new_rejects_empty_source() {
        // ARRANGE / ACT
        let err = OverlaySpec::new("name".into(), String::new()).expect_err("should fail");

        // ASSERT
        assert!(matches!(err, WizardError::ProfileValidation(_)));
    }

    #[test]
    fn customization_spec_new_rejects_empty_extension() {
        // ARRANGE / ACT
        let err = CustomizationSpec::new(vec![String::new()]).expect_err("should fail");

        // ASSERT
        assert!(matches!(err, WizardError::ProfileValidation(_)));
    }

    #[test]
    fn document_new_accepts_valid_profile() {
        // ARRANGE
        let overlay = OverlaySpec::new("name".into(), "source".into()).expect("valid overlay");
        let customization = CustomizationSpec::new(vec![]).expect("valid customization");
        let kernel = KernelSpec::new("muak-os/linux".to_owned()).expect("valid kernel");
        // ACT
        let doc = Profile::new(Some(overlay), customization, kernel);
        let id = doc.profile_id().expect("id");

        // ASSERT
        assert_eq!(id.to_string().len(), 64);
    }

    #[test]
    fn document_new_accepts_no_overlay() {
        // ARRANGE
        let customization = CustomizationSpec::new(vec![]).expect("valid customization");
        let kernel = KernelSpec::new("muak-os/linux".to_owned()).expect("valid kernel");
        // ACT
        let doc = Profile::new(None, customization, kernel);
        let id = doc.profile_id().expect("id");

        // ASSERT
        assert_eq!(id.to_string().len(), 64);
    }
}
