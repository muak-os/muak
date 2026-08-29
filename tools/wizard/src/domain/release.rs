//! Canonical release manifest that contains logical sources mapped to repository tags.

use serde::{Deserialize, Serialize};

use crate::domain::identity::{RELEASE_API_VERSION, ReleaseManifestId};
use crate::domain::{canonical_toml, non_empty};
use crate::error::{Result, WizardError};

/// TODO: replace this with published OCI release manifest.
const MANIFEST_TOML: &str = r#"
api_version = "muak-release-v1"
name = "muak-os/release"
version = "latest"

[installer]
source = "muak-os/installer"
repository = "installer"
tag = "latest"

[stub]
source = "muak-os/stub"
repository = "stub"
tag = "latest"

[kernel]
source = "muak-os/linux"
repository = "linux"
tag = "latest"

[[extensions]]
name = "qemu"
source = "muak-os/qemu"
repository = "pkgs/qemu"
tag = "latest"

[[overlays]]
name = "rpi_generic"
source = "muak-os/sbc-raspberrypi"
repository = "sbc/raspberrypi"
tag = "latest"

[[overlays]]
name = "rpi_5"
source = "muak-os/sbc-raspberrypi"
repository = "sbc/raspberrypi-5"
tag = "latest"
"#;

/// Returns the embedded development release manifest at its base `latest` tag.
///
/// # Errors
///
/// Returns an error when the embedded manifest fails to parse or validate.
pub fn manifest() -> Result<Manifest> {
    Manifest::from_toml(MANIFEST_TOML.as_bytes())
}

/// Returns the embedded development release manifest with its version and all
/// source tags set to `version`.
///
/// TODO: remove when the manifest is published to OCI and pulled per version.
///
/// # Errors
///
/// Returns an error when `version` is empty or contains characters that could
/// corrupt an OCI reference, or when the embedded manifest fails to parse or
/// validate.
pub fn manifest_for_version(version: &str) -> Result<Manifest> {
    valid_version(version)?;
    manifest()?.with_version(version)
}

/// Rejects versions that cannot name a release or would corrupt an OCI
/// reference.
///
/// # Errors
///
/// Returns an error when `version` is empty or contains whitespace or the
/// reference-delimiting `:` or `/` characters.
fn valid_version(version: &str) -> Result<()> {
    if version.is_empty()
        || version
            .chars()
            .any(|ch| ch.is_whitespace() || ch == ':' || ch == '/')
    {
        return Err(WizardError::SourceResolution(format!(
            "invalid release version: '{version}'"
        )));
    }

    Ok(())
}

/// The canonical source bundle for one release.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Manifest {
    api_version: String,
    name: String,
    version: String,
    installer: Image,
    stub: Image,
    kernel: Image,
    #[serde(default)]
    extensions: Vec<Extension>,
    #[serde(default)]
    overlays: Vec<Overlay>,
}

impl Manifest {
    /// Deserializes and validates a manifest from TOML bytes.
    ///
    /// # Errors
    ///
    /// Returns an error when parsing or semantic validation fails.
    pub fn from_toml(bytes: &[u8]) -> Result<Self> {
        let manifest: Self = toml::from_str(core::str::from_utf8(bytes).map_err(|_error| {
            WizardError::ProfileValidation("release manifest is not valid UTF-8".into())
        })?)
        .map_err(|e| {
            WizardError::ProfileValidation(format!("failed to parse release manifest TOML: {e}"))
        })?;
        manifest.validate()?;

        Ok(manifest)
    }

    /// Returns the release catalog family name.
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Returns the release version.
    #[must_use]
    pub fn version(&self) -> &str {
        &self.version
    }

    /// Returns the installer image entry.
    #[must_use]
    pub const fn installer(&self) -> &Image {
        &self.installer
    }

    /// Returns the stub image entry.
    #[must_use]
    pub const fn stub(&self) -> &Image {
        &self.stub
    }

    /// Returns the kernel image entry.
    #[must_use]
    pub const fn kernel(&self) -> &Image {
        &self.kernel
    }

    /// Returns the available extensions.
    #[must_use]
    pub fn extensions(&self) -> &[Extension] {
        &self.extensions
    }

    /// Returns the available overlays.
    #[must_use]
    pub fn overlays(&self) -> &[Overlay] {
        &self.overlays
    }

    /// Computes the stable release manifest identity.
    ///
    /// # Errors
    ///
    /// Returns an error when canonical serialization fails.
    pub fn id(&self) -> Result<ReleaseManifestId> {
        Ok(ReleaseManifestId::new(&self.canonical_bytes()?))
    }

    /// Serializes the manifest to canonical TOML bytes with sorted entries.
    ///
    /// # Errors
    ///
    /// Returns an error when TOML serialization fails.
    pub fn canonical_bytes(&self) -> Result<Vec<u8>> {
        let mut normalized = self.clone();
        normalized
            .extensions
            .sort_by(|left, right| left.source.cmp(&right.source));
        normalized
            .overlays
            .sort_by(|left, right| left.source.cmp(&right.source));

        canonical_toml(&normalized)
    }

    /// Returns a copy of this manifest with its version and every source tag
    /// set to `version`.
    ///
    /// TODO: remove when the manifest is published to OCI.
    fn with_version(&self, version: &str) -> Result<Self> {
        fn set_tag(entry: &mut String, version: &str) {
            version.clone_into(entry);
        }

        let mut manifest = self.clone();
        version.clone_into(&mut manifest.version);
        set_tag(&mut manifest.installer.tag, version);
        set_tag(&mut manifest.stub.tag, version);
        set_tag(&mut manifest.kernel.tag, version);
        for entry in &mut manifest.extensions {
            set_tag(&mut entry.tag, version);
        }
        for entry in &mut manifest.overlays {
            set_tag(&mut entry.tag, version);
        }

        manifest.validate()?;

        Ok(manifest)
    }

    /// Checks the manifest against the supported contract.
    ///
    /// # Errors
    ///
    /// Returns an error when the API version is unsupported.
    fn validate(&self) -> Result<()> {
        if self.api_version != RELEASE_API_VERSION {
            return Err(WizardError::ProfileValidation(format!(
                "unsupported release manifest API version: {}",
                self.api_version
            )));
        }

        Ok(())
    }
}

/// A source image entry with a repository path and a mutable tag.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Image {
    #[serde(deserialize_with = "non_empty")]
    source: String,
    #[serde(deserialize_with = "non_empty")]
    repository: String,
    #[serde(deserialize_with = "non_empty")]
    tag: String,
}

impl Image {
    /// Returns the logical source identity.
    #[must_use]
    pub fn source(&self) -> &str {
        &self.source
    }

    /// Returns the repository path relative to the registry prefix.
    #[must_use]
    pub fn repository(&self) -> &str {
        &self.repository
    }

    /// Returns the mutable tag.
    #[must_use]
    pub fn tag(&self) -> &str {
        &self.tag
    }

    /// Returns the registry-qualified OCI reference for this image entry.
    #[must_use]
    pub fn reference(&self, registry: &str) -> String {
        format!("{registry}/{}:{}", self.repository, self.tag)
    }
}

/// A named extension image entry.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Extension {
    #[serde(deserialize_with = "non_empty")]
    name: String,
    #[serde(deserialize_with = "non_empty")]
    source: String,
    #[serde(deserialize_with = "non_empty")]
    repository: String,
    #[serde(deserialize_with = "non_empty")]
    tag: String,
}

impl Extension {
    /// Returns the short extension name.
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Returns the logical source identity.
    #[must_use]
    pub fn source(&self) -> &str {
        &self.source
    }

    /// Returns the repository path relative to the registry prefix.
    #[must_use]
    pub fn repository(&self) -> &str {
        &self.repository
    }

    /// Returns the mutable tag.
    #[must_use]
    pub fn tag(&self) -> &str {
        &self.tag
    }

    /// Returns the registry-qualified OCI reference for this extension entry.
    #[must_use]
    pub fn reference(&self, registry: &str) -> String {
        format!("{registry}/{}:{}", self.repository, self.tag)
    }
}

/// A named overlay image entry.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Overlay {
    #[serde(deserialize_with = "non_empty")]
    name: String,
    #[serde(deserialize_with = "non_empty")]
    source: String,
    #[serde(deserialize_with = "non_empty")]
    repository: String,
    #[serde(deserialize_with = "non_empty")]
    tag: String,
}

impl Overlay {
    /// Returns the overlay name used as the image path prefix.
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Returns the logical source identity.
    #[must_use]
    pub fn source(&self) -> &str {
        &self.source
    }

    /// Returns the repository path relative to the registry prefix.
    #[must_use]
    pub fn repository(&self) -> &str {
        &self.repository
    }

    /// Returns the mutable tag.
    #[must_use]
    pub fn tag(&self) -> &str {
        &self.tag
    }

    /// Returns the registry-qualified OCI reference for this overlay entry.
    #[must_use]
    pub fn reference(&self, registry: &str) -> String {
        format!("{registry}/{}:{}", self.repository, self.tag)
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn manifest_name_matches_default_family() {
        // ARRANGE / ACT
        let manifest = super::manifest().expect("manifest");

        // ASSERT
        assert_eq!(manifest.name(), "muak-os/release");
    }

    use super::*;

    const MANIFEST: &str = r#"
api_version = "muak-release-v1"
name = "muak-os/release"
version = "v1.0.0"

[installer]
source = "muak-os/installer"
repository = "installer"
tag = "v1.0.0"

[stub]
source = "muak-os/stub"
repository = "stub"
tag = "v1.0.0"

[kernel]
source = "muak-os/linux"
repository = "linux"
tag = "v1.0.0"

[[extensions]]
name = "qemu"
source = "muak-os/qemu"
repository = "pkgs/qemu"
tag = "v1.0.0"

[[overlays]]
name = "rpi_generic"
source = "muak-os/sbc-raspberrypi"
repository = "sbc/raspberrypi"
tag = "v1.0.0"
"#;

    #[test]
    fn hardcoded_manifest_parses_and_validates() {
        // ARRANGE / ACT
        let manifest = manifest().expect("hardcoded manifest");

        // ASSERT
        assert_eq!(manifest.name(), "muak-os/release");
        assert_eq!(manifest.installer().repository(), "installer");
        assert_eq!(manifest.stub().repository(), "stub");
        assert_eq!(manifest.kernel().repository(), "linux");
        assert_eq!(manifest.extensions().len(), 1);
        assert_eq!(manifest.overlays().len(), 2);
        assert_eq!(
            manifest.overlays().first().expect("overlay").repository(),
            "sbc/raspberrypi"
        );
        assert_eq!(
            manifest.overlays().get(1).expect("overlay").repository(),
            "sbc/raspberrypi-5"
        );
    }

    #[test]
    fn manifest_for_version_sets_version_and_all_tags() {
        // ARRANGE / ACT
        let manifest = manifest_for_version("v2.3.4").expect("manifest for version");

        // ASSERT
        assert_eq!(manifest.version(), "v2.3.4");
        assert_eq!(manifest.installer().tag(), "v2.3.4");
        assert_eq!(manifest.stub().tag(), "v2.3.4");
        assert_eq!(manifest.kernel().tag(), "v2.3.4");
        assert!(
            manifest
                .extensions()
                .iter()
                .all(|entry| entry.tag() == "v2.3.4"),
            "every extension tag should follow the version"
        );
        assert!(
            manifest
                .overlays()
                .iter()
                .all(|entry| entry.tag() == "v2.3.4"),
            "every overlay tag should follow the version"
        );
    }

    #[test]
    fn manifest_for_version_rejects_invalid_versions() {
        // ARRANGE / ACT / ASSERT
        for version in ["", "v1:latest", "v1/latest", "v1 latest"] {
            assert!(
                manifest_for_version(version).is_err(),
                "expected '{version}' to be rejected"
            );
        }
    }

    #[test]
    fn parses_manifest() {
        // ARRANGE / ACT
        let manifest = Manifest::from_toml(MANIFEST.as_bytes()).expect("parse");

        // ASSERT
        assert_eq!(manifest.version(), "v1.0.0");
        assert_eq!(manifest.installer().source(), "muak-os/installer");
        assert_eq!(manifest.stub().source(), "muak-os/stub");
        assert_eq!(manifest.kernel().tag(), "v1.0.0");
        assert_eq!(manifest.extensions().first().expect("ext").name(), "qemu");
        assert_eq!(
            manifest.overlays().first().expect("overlay").name(),
            "rpi_generic"
        );
    }

    #[test]
    fn manifest_id_is_stable() {
        // ARRANGE
        let manifest = Manifest::from_toml(MANIFEST.as_bytes()).expect("parse");

        // ACT
        let id1 = manifest.id().expect("id");
        let id2 = manifest.id().expect("id");

        // ASSERT
        assert_eq!(id1, id2);
        assert_eq!(id1.to_string().len(), 64);
    }

    #[test]
    fn manifest_id_ignores_entry_order() {
        // ARRANGE
        let mut manifest = Manifest::from_toml(MANIFEST.as_bytes()).expect("parse");
        manifest.extensions.push(Extension {
            name: "b".into(),
            source: "muak-os/b".into(),
            repository: "pkgs/b".into(),
            tag: "v1.0.0".into(),
        });
        let mut reversed = manifest.clone();
        reversed.extensions.reverse();

        // ACT
        let id1 = manifest.id().expect("id");
        let id2 = reversed.id().expect("id");

        // ASSERT
        assert_eq!(id1, id2);
    }

    #[test]
    fn rejects_unknown_fields() {
        // ARRANGE
        let raw = b"unknown_key = true\napi_version = \"muak-release-v1\"\nname = \"r\"\nversion = \"v1\"\n[installer]\nsource = \"s\"\nrepository = \"r\"\ntag = \"t\"\n[kernel]\nsource = \"s\"\nrepository = \"r\"\ntag = \"t\"";

        // ACT
        let err = Manifest::from_toml(raw).expect_err("should fail");

        // ASSERT
        assert!(matches!(err, WizardError::ProfileValidation(_)));
    }

    #[test]
    fn rejects_unsupported_api_version() {
        // ARRANGE
        let raw = b"api_version = \"other\"\nname = \"r\"\nversion = \"v1\"\n[installer]\nsource = \"s\"\nrepository = \"r\"\ntag = \"t\"\n[kernel]\nsource = \"s\"\nrepository = \"r\"\ntag = \"t\"";

        // ACT
        let err = Manifest::from_toml(raw).expect_err("should fail");

        // ASSERT
        assert!(matches!(err, WizardError::ProfileValidation(_)));
    }

    #[test]
    fn rejects_empty_repository() {
        // ARRANGE
        let raw = b"api_version = \"muak-release-v1\"\nname = \"r\"\nversion = \"v1\"\n[installer]\nsource = \"s\"\nrepository = \"\"\ntag = \"t\"\n[kernel]\nsource = \"s\"\nrepository = \"r\"\ntag = \"t\"";

        // ACT
        let err = Manifest::from_toml(raw).expect_err("should fail");

        // ASSERT
        assert!(matches!(err, WizardError::ProfileValidation(_)));
    }

    #[test]
    fn rejects_missing_installer() {
        // ARRANGE
        let raw = b"api_version = \"muak-release-v1\"\nname = \"r\"\nversion = \"v1\"\n[kernel]\nsource = \"s\"\nrepository = \"r\"\ntag = \"t\"";

        // ACT
        let err = Manifest::from_toml(raw).expect_err("should fail");

        // ASSERT
        assert!(matches!(err, WizardError::ProfileValidation(_)));
    }

    #[test]
    fn default_extensions_and_overlays_are_empty() {
        // ARRANGE
        let raw = b"api_version = \"muak-release-v1\"\nname = \"r\"\nversion = \"v1\"\n[installer]\nsource = \"s\"\nrepository = \"r\"\ntag = \"t\"\n[stub]\nsource = \"s\"\nrepository = \"r\"\ntag = \"t\"\n[kernel]\nsource = \"s\"\nrepository = \"r\"\ntag = \"t\"";

        // ACT
        let manifest = Manifest::from_toml(raw).expect("parse");

        // ASSERT
        assert!(manifest.extensions().is_empty());
        assert!(manifest.overlays().is_empty());
    }
}
