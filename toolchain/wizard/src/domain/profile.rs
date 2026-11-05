//! The version-neutral, content-addressed customization spec.

use serde::{Deserialize, Deserializer, Serialize};

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

    /// Serializes the profile with sorted extensions to canonical bytes.
    ///
    /// # Errors
    ///
    /// Returns an error when extension sorting or serialization fails.
    pub fn canonical_bytes(&self) -> Result<Vec<u8>> {
        let mut normalized = self.clone();
        normalized.customization.extensions = canonical_extensions(&self.customization.extensions)?;

        canonical_toml(&normalized)
    }
}

/// Optional overlay that selects a board-specific boot asset set from an OCI image.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OverlaySpec {
    #[serde(deserialize_with = "non_empty")]
    name: String,
}

impl OverlaySpec {
    /// Creates a validated overlay spec.
    ///
    /// # Errors
    ///
    /// Returns an error when `name` is empty.
    pub fn new(name: String) -> Result<Self> {
        reject_empty(&name, "overlay.name")?;

        Ok(Self { name })
    }

    /// Returns the overlay name.
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }
}

/// Kernel package source and ordered additional boot parameters.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct KernelSpec {
    #[serde(deserialize_with = "non_empty")]
    source: String,
    #[serde(
        default,
        skip_serializing_if = "Vec::is_empty",
        deserialize_with = "kernel_parameters_append"
    )]
    parameters_append: Vec<String>,
}

impl KernelSpec {
    /// Creates a validated kernel spec.
    ///
    /// # Errors
    ///
    /// Returns an error when `source` is empty.
    pub fn new(source: String) -> Result<Self> {
        reject_empty(&source, "kernel.source")?;

        Ok(Self {
            source,
            parameters_append: Vec::new(),
        })
    }

    /// Adds parameters after the kernel package's command line, in the given order.
    /// Values containing spaces must use Linux double-quote syntax.
    ///
    /// # Errors
    ///
    /// Returns an error for an empty parameter, control characters, unquoted
    /// whitespace, unmatched double quotes, or the init-argument separator `--`.
    pub fn with_parameters_append(mut self, parameters: Vec<String>) -> Result<Self> {
        validate_kernel_parameters_append(&parameters)?;
        self.parameters_append = parameters;

        Ok(self)
    }

    /// Returns the logical kernel source identity.
    #[must_use]
    pub fn source(&self) -> &str {
        &self.source
    }

    /// Returns additional parameters in command-line order, including duplicates.
    #[must_use]
    pub fn parameters_append(&self) -> &[String] {
        &self.parameters_append
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

fn kernel_parameters_append<'de, D: Deserializer<'de>>(
    deserializer: D,
) -> Result<Vec<String>, D::Error> {
    let parameters = Vec::<String>::deserialize(deserializer)?;
    validate_kernel_parameters_append(&parameters).map_err(serde::de::Error::custom)?;

    Ok(parameters)
}

fn validate_kernel_parameters_append(parameters: &[String]) -> Result<()> {
    for parameter in parameters {
        let mut quoted = false;
        let mut invalid = parameter.trim_matches('"').is_empty()
            || parameter
                .chars()
                .filter(|character| *character != '"')
                .eq(['-', '-']);
        for character in parameter.chars() {
            quoted ^= character == '"';
            invalid |= character.is_control() || (character.is_whitespace() && !quoted);
        }
        if invalid || quoted {
            return Err(WizardError::ProfileValidation(
                "kernel.parameters_append must contain individual non-empty parameters with balanced double quotes, no control characters or unquoted whitespace, and no -- separator".to_owned(),
            ));
        }
    }

    Ok(())
}

/// Sorts extension names and rejects duplicates.
///
/// # Errors
///
/// Returns an error when two names normalize to the same identity.
fn canonical_extensions(names: &[String]) -> Result<Vec<String>> {
    let mut names = names.to_vec();
    names.sort_unstable();

    if names.windows(2).any(|pair| pair.first() == pair.get(1)) {
        return Err(WizardError::ProfileValidation(
            "duplicate extension name".to_owned(),
        ));
    }

    Ok(names)
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

    fn overlay_toml() -> &'static str {
        r#"
[overlay]
name = "rpi_generic"

[kernel]
source = "muak-os/linux"

[customization]
extensions = ["qemu"]
"#
    }

    #[test]
    fn parses_minimal_profile() {
        // ARRANGE / ACT
        let doc = Profile::from_toml(minimal_toml().as_bytes()).expect("parse");

        // ASSERT
        assert!(doc.overlay().is_none());
        assert_eq!(doc.kernel().source(), "muak-os/linux");
        assert_eq!(doc.customization().extensions(), Vec::<String>::new());
    }

    #[test]
    fn empty_parameters_preserve_the_existing_profile_identity() {
        // ARRANGE
        let profile = Profile::from_toml(minimal_toml().as_bytes()).expect("parse");
        let explicit = minimal_toml().replace(
            "source = \"muak-os/linux\"",
            "source = \"muak-os/linux\"\nparameters_append = []",
        );
        let explicit = Profile::from_toml(explicit.as_bytes()).expect("parse explicit empty");

        // ACT / ASSERT
        for profile in [profile, explicit] {
            assert_eq!(profile.kernel().parameters_append(), Vec::<String>::new());
            assert_eq!(
                profile.profile_id().expect("id").to_string(),
                "5b94fd5d6a48980e85acaa357fdcebf7e675ea3a104b3d6a43328d599bab71b5",
                "profiles without parameters must retain their pre-customization identity"
            );
        }
    }

    #[test]
    fn parameters_roundtrip_with_quoted_values_and_keep_duplicates_in_order() {
        // ARRANGE
        let parameters = vec![
            "console=ttyS0".to_owned(),
            "console=tty0".to_owned(),
            "test.message=\"hello world\"".to_owned(),
        ];
        let profile = Profile::new(
            None,
            CustomizationSpec::new(Vec::new()).expect("customization"),
            KernelSpec::new("muak-os/linux".to_owned())
                .expect("kernel")
                .with_parameters_append(parameters.clone())
                .expect("parameters"),
        );

        // ACT
        let restored =
            Profile::from_toml(&profile.canonical_bytes().expect("serialize")).expect("restore");

        // ASSERT
        assert_eq!(restored.kernel().parameters_append(), parameters);
        assert_eq!(profile, restored);
        let mut reversed = profile.clone();
        reversed.kernel.parameters_append.reverse();
        let base = Profile::from_toml(minimal_toml().as_bytes()).expect("base");
        assert_ne!(
            profile.profile_id().expect("id"),
            base.profile_id().expect("base id")
        );
        assert_ne!(
            profile.profile_id().expect("id"),
            reversed.profile_id().expect("reversed id")
        );
    }

    #[test]
    fn invalid_parameters_are_rejected_by_both_the_builder_and_toml() {
        // ARRANGE
        for parameter in [
            "",
            "\"\"",
            "--",
            "\"--\"",
            "foo bar",
            "foo\tbar",
            "foo\nbar",
            "foo\0bar",
            "foo=\"unclosed",
        ] {
            let parameters = vec![parameter.to_owned()];
            let kernel = KernelSpec::new("muak-os/linux".to_owned()).expect("kernel");
            let mut raw = Profile::from_toml(minimal_toml().as_bytes()).expect("profile");
            raw.kernel.parameters_append = parameters.clone();

            // ACT / ASSERT
            kernel
                .with_parameters_append(parameters)
                .expect_err("invalid parameter builder");
            Profile::from_toml(&raw.canonical_bytes().expect("serialize"))
                .expect_err("invalid parameter TOML");
        }
    }

    #[test]
    fn parses_overlay_profile() {
        // ARRANGE / ACT
        let doc = Profile::from_toml(overlay_toml().as_bytes()).expect("parse");

        // ASSERT
        let ov = doc.overlay().expect("overlay present");
        assert_eq!(ov.name(), "rpi_generic");
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
            b"[overlay]\nname = \"rpi_5\"\n[kernel]\nsource = \"muak-os/linux\"\n[customization]\nextensions = []",
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
            b"[kernel]\nsource = \"muak-os/linux\"\n[customization]\nextensions = [\"qemu\", \"qemu\"]",
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
        let err = OverlaySpec::new(String::new()).expect_err("should fail");

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
        let overlay = OverlaySpec::new("name".into()).expect("valid overlay");
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
