//! Document parsing and release-line validation.

use core::str;

use crate::error;
use crate::schema::documents::Document;
use crate::schema::kinds::Kind;

/// Parses and validates the document of `kind` for `release` from TOML bytes.
///
/// # Errors
///
/// Returns an error when the bytes are not UTF-8, fail to parse, or the
/// document carries an unexpected `api_version` or `release`.
pub fn from_toml(kind: Kind, bytes: &[u8], release: &str) -> error::DocumentResult<Document> {
    let text = str::from_utf8(bytes).map_err(|_error| {
        error::DocumentError::Document(format!(
            "{} catalog for '{release}' is not valid UTF-8",
            kind.dir()
        ))
    })?;
    let document = match kind {
        Kind::Core => Document::Core(toml::from_str(text).map_err(error::DocumentError::from)?),
        Kind::Overlays => {
            Document::Overlays(toml::from_str(text).map_err(error::DocumentError::from)?)
        }
        Kind::Extensions => {
            Document::Extensions(toml::from_str(text).map_err(error::DocumentError::from)?)
        }
    };
    validate(&document, kind, release)?;

    Ok(document)
}

/// Rejects releases that cannot name a catalog tag or document path.
///
/// # Errors
///
/// Returns an error when `release` is empty or contains whitespace or the
/// reference-delimiting `:` or `/` characters.
pub fn validate_release(release: &str) -> error::DocumentResult<()> {
    if release.is_empty()
        || release
            .chars()
            .any(|ch| ch.is_whitespace() || ch == ':' || ch == '/')
    {
        return Err(error::DocumentError::Document(format!(
            "invalid release version: '{release}'"
        )));
    }

    Ok(())
}

fn validate(document: &Document, kind: Kind, release: &str) -> error::DocumentResult<()> {
    if document.api_version() != kind.api_version() {
        return Err(error::DocumentError::Document(format!(
            "unsupported api_version '{}' (expected '{}')",
            document.api_version(),
            kind.api_version()
        )));
    }
    if document.release() != release {
        return Err(error::DocumentError::Document(format!(
            "document release '{}' does not match '{release}'",
            document.release()
        )));
    }
    check_duplicates(document)?;

    Ok(())
}

fn check_duplicates(document: &Document) -> error::DocumentResult<()> {
    match *document {
        Document::Core(ref core) => check(
            "kernel source",
            core.kernels.iter().map(|entry| entry.source.as_str()),
        )?,
        Document::Overlays(ref overlays) => check(
            "overlay name",
            overlays.overlays.iter().map(|entry| entry.name.as_str()),
        )?,
        Document::Extensions(ref extensions) => check(
            "extension name",
            extensions
                .extensions
                .iter()
                .map(|entry| entry.name.as_str()),
        )?,
    }

    Ok(())
}

fn check<'a>(kind: &str, identities: impl Iterator<Item = &'a str>) -> error::DocumentResult<()> {
    let mut seen: Vec<&'a str> = Vec::new();
    for identity in identities {
        if seen.contains(&identity) {
            return Err(error::DocumentError::Document(format!(
                "duplicate {kind} '{identity}'"
            )));
        }
        seen.push(identity);
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const CORE: &str = r#"api_version = "muak.dev/catalog/core/v1"
release = "v1.2.3"

[[kernels]]
source = "muak-os/linux"
repository = "linux"
tag = "v6.12.4-muak1"
digest = "sha256:1111"

[stub]
source = "muak-os/stub"
repository = "stub"
tag = "v0.3.1"
digest = "sha256:2222"

[installer]
source = "muak-os/installer"
repository = "installer"
tag = "v1.2.3"
digest = "sha256:3333"
"#;

    #[test]
    fn from_toml_parses_and_resolves_core_lookups() {
        // ARRANGE / ACT
        let document = from_toml(Kind::Core, CORE.as_bytes(), "v1.2.3").expect("parse core");

        // ASSERT
        assert_eq!(document.kind(), Kind::Core);
        assert_eq!(
            document.kernel("muak-os/linux").expect("kernel").digest,
            "sha256:1111"
        );
        assert_eq!(document.stub().expect("stub").digest, "sha256:2222");
        assert_eq!(
            document.installer().expect("installer").digest,
            "sha256:3333"
        );
        document.kernel("other/kernel").unwrap_err();
    }

    #[test]
    fn from_toml_rejects_unknown_api_version_and_release_mismatch() {
        // ARRANGE
        let stale = CORE.replace("release = \"v1.2.3\"", "release = \"v1.2.2\"");

        // ACT / ASSERT
        let error = from_toml(
            Kind::Core,
            b"api_version = \"other\"\nrelease = \"v\"\n",
            "v1.2.3",
        )
        .expect_err("unknown api_version should fail");
        assert!(error.to_string().contains("unsupported api_version"));

        let error = from_toml(Kind::Core, stale.as_bytes(), "v1.2.3")
            .expect_err("release mismatch should fail");
        assert!(error.to_string().contains("does not match"));
    }

    #[test]
    fn from_toml_rejects_duplicate_named_entries() {
        // ARRANGE
        let raw = r#"api_version = "muak.dev/catalog/overlays/v1"
release = "v1.2.3"

[[overlays]]
name = "rpi_generic"
source = "muak-os/sbc-raspberrypi"
repository = "sbc/raspberrypi"
tag = "v0.4.0"
digest = "sha256:5555"

[[overlays]]
name = "rpi_generic"
source = "acme/sbc"
repository = "acme/rpi"
tag = "v9.9.9"
digest = "sha256:6666"
"#;

        // ACT / ASSERT
        let error = from_toml(Kind::Overlays, raw.as_bytes(), "v1.2.3")
            .expect_err("duplicate overlay name should fail");
        assert!(
            error
                .to_string()
                .contains("duplicate overlay name 'rpi_generic'")
        );
    }

    #[test]
    fn from_toml_rejects_duplicate_kernel_sources() {
        // ARRANGE
        let raw = r#"api_version = "muak.dev/catalog/core/v1"
release = "v1.2.3"

[[kernels]]
source = "muak-os/linux"
repository = "linux"
tag = "v6.12.4-muak1"
digest = "sha256:1111"

[[kernels]]
source = "muak-os/linux"
repository = "linux"
tag = "v6.12.4-muak1"
digest = "sha256:2222"
"#;

        // ACT / ASSERT
        let error = from_toml(Kind::Core, raw.as_bytes(), "v1.2.3")
            .expect_err("duplicate kernel source should fail");
        assert!(
            error
                .to_string()
                .contains("duplicate kernel source 'muak-os/linux'")
        );
    }

    #[test]
    fn from_toml_rejects_unknown_fields() {
        // ARRANGE
        let raw =
            "api_version = \"muak.dev/catalog/core/v1\"\nrelease = \"v1.2.3\"\nunknown_key = true";

        // ACT / ASSERT
        let error =
            from_toml(Kind::Core, raw.as_bytes(), "v1.2.3").expect_err("unknown field should fail");
        assert!(matches!(error, error::DocumentError::Parse(_)));
    }
}
