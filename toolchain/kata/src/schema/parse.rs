//! Document parsing and release-line validation.

use core::str;

use semver::Version;

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
    let document = decode(kind, bytes, release)?;
    validate(&document, kind, Some(release))?;

    Ok(document)
}

/// Parses and validates the document of `kind` fetched at an arbitrary tag.
///
/// # Errors
///
/// Returns an error when the bytes are not UTF-8, fail to parse, or the
/// document carries an unexpected `api_version`.
pub fn from_toml_tag(kind: Kind, bytes: &[u8], tag: &str) -> error::DocumentResult<Document> {
    let document = decode(kind, bytes, tag)?;
    validate(&document, kind, None)?;

    Ok(document)
}

fn decode(kind: Kind, bytes: &[u8], context: &str) -> error::DocumentResult<Document> {
    let text = str::from_utf8(bytes).map_err(|_error| {
        error::DocumentError::Document(format!(
            "{} catalog for '{context}' is not valid UTF-8",
            kind.dir()
        ))
    })?;
    Ok(match kind {
        Kind::Core => Document::Core(toml::from_str(text).map_err(error::DocumentError::from)?),
        Kind::Overlays => {
            Document::Overlays(toml::from_str(text).map_err(error::DocumentError::from)?)
        }
        Kind::Extensions => {
            Document::Extensions(toml::from_str(text).map_err(error::DocumentError::from)?)
        }
    })
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

/// Parses a release line into a semantic version (`v` prefix optional).
///
/// # Errors
///
/// Returns an error when `release` is not a valid semantic version.
pub fn release_version(release: &str) -> error::DocumentResult<Version> {
    let core = release.strip_prefix('v').unwrap_or(release);

    Version::parse(core).map_err(|error| {
        error::DocumentError::Document(format!("invalid release '{release}': {error}"))
    })
}

fn validate(document: &Document, kind: Kind, release: Option<&str>) -> error::DocumentResult<()> {
    if document.api_version() != kind.api_version() {
        return Err(error::DocumentError::Document(format!(
            "unsupported api_version '{}' (expected '{}')",
            document.api_version(),
            kind.api_version()
        )));
    }
    if let Some(release) = release
        && document.release() != release
    {
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
    fn from_toml_tag_accepts_a_tag_different_from_the_release() {
        // ARRANGE / ACT
        let document = from_toml_tag(Kind::Core, CORE.as_bytes(), "stable").expect("parse core");

        // ASSERT
        assert_eq!(document.release(), "v1.2.3");
    }

    #[test]
    fn from_toml_tag_still_validates_api_version_and_duplicates() {
        // ARRANGE
        let wrong_api = CORE.replace("muak.dev/catalog/core/v1", "muak.dev/catalog/core/v0");
        let duplicate = CORE.replace(
            "[installer]",
            "[[kernels]]\nsource = \"muak-os/linux\"\nrepository = \"linux\"\ntag = \"v6\"\ndigest = \"sha256:4444\"\n\n[installer]",
        );

        // ACT / ASSERT
        let error = from_toml_tag(Kind::Core, wrong_api.as_bytes(), "stable")
            .expect_err("wrong api_version must fail");
        assert!(error.to_string().contains("unsupported api_version"));
        from_toml_tag(Kind::Core, duplicate.as_bytes(), "stable")
            .expect_err("duplicate kernel must fail");
    }

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
