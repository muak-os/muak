//! Schema-tolerant lifting of individual fields from a config document.
//!
//! These helpers read a few top-level fields without deserializing into
//! [`crate::system::Config`], so a document written for another `api_version`
//! still answers them.

use crate::error::{ConfigError, Result};

/// `network` subtree and `host.port`, compared by their canonical rendering.
#[derive(Debug, PartialEq, Eq)]
pub struct Reachability {
    network: Option<String>,
    port: Option<String>,
}

/// Returns `host.version`, or `None` when it is absent or empty.
///
/// # Errors
///
/// Returns [`ConfigError::ParseError`] when the document is not TOML, and
/// [`ConfigError::ValidationError`] when the field is not a string.
pub fn version(raw: &str) -> Result<Option<String>> {
    let document = parse(raw)?;

    text(host_field(&document, "version"), "host.version")
}

/// Returns `host.registry`, or `None` when it is absent or empty.
///
/// # Errors
///
/// Returns [`ConfigError::ParseError`] when the document is not TOML, and
/// [`ConfigError::ValidationError`] when the field is not a string.
pub fn registry(raw: &str) -> Result<Option<String>> {
    let document = parse(raw)?;

    text(host_field(&document, "registry"), "host.registry")
}

/// Returns `host.secureboot`, or `None` when it is absent.
///
/// # Errors
///
/// Returns [`ConfigError::ParseError`] when the document is not TOML, and
/// [`ConfigError::ValidationError`] when the field is not a boolean.
pub fn secureboot(raw: &str) -> Result<Option<bool>> {
    let document = parse(raw)?;
    let Some(value) = host_field(&document, "secureboot") else {
        return Ok(None);
    };

    value.as_bool().map(Some).ok_or_else(|| {
        ConfigError::ValidationError(format!(
            "host.secureboot must be a boolean, found {}",
            value.type_str()
        ))
    })
}

/// Returns `host.extensions`, or an empty list when it is absent.
///
/// # Errors
///
/// Returns [`ConfigError::ParseError`] when the document is not TOML, and
/// [`ConfigError::ValidationError`] when the field is not an array of strings.
pub fn extensions(raw: &str) -> Result<Vec<String>> {
    let document = parse(raw)?;
    let Some(value) = host_field(&document, "extensions") else {
        return Ok(Vec::new());
    };
    let items = value.as_array().ok_or_else(|| {
        ConfigError::ValidationError(format!(
            "host.extensions must be an array, found {}",
            value.type_str()
        ))
    })?;

    items
        .iter()
        .map(|item| {
            item.as_str().map(str::to_owned).ok_or_else(|| {
                ConfigError::ValidationError(format!(
                    "host.extensions entries must be strings, found {}",
                    item.type_str()
                ))
            })
        })
        .collect()
}

/// Lifts the `network` subtree and `host.port` for reachability comparison.
///
/// # Errors
///
/// Returns [`ConfigError::ParseError`] when the document is not TOML.
pub fn reachability(raw: &str) -> Result<Reachability> {
    let document = parse(raw)?;

    Ok(Reachability {
        network: document.get("network").map(ToString::to_string),
        port: host_field(&document, "port").map(ToString::to_string),
    })
}

fn parse(raw: &str) -> Result<toml::Value> {
    Ok(toml::from_str(raw)?)
}

fn host_field<'a>(document: &'a toml::Value, field: &str) -> Option<&'a toml::Value> {
    document.get("host")?.get(field)
}

fn text(value: Option<&toml::Value>, field: &str) -> Result<Option<String>> {
    let Some(value) = value else {
        return Ok(None);
    };
    let text = value.as_str().ok_or_else(|| {
        ConfigError::ValidationError(format!(
            "{field} must be a string, found {}",
            value.type_str()
        ))
    })?;

    Ok((!text.is_empty()).then(|| text.to_owned()))
}

#[cfg(test)]
mod tests {
    use super::{extensions, reachability, registry, secureboot, version};
    use crate::error::ConfigError;

    const MINIMAL: &str = r#"
        [host]
        registry = "ghcr.io/muak-os"
        version = "v1.2.0"
        extensions = ["gpu"]
        secureboot = true

        [network]
        dns = ["1.1.1.1"]
    "#;

    #[test]
    fn minimal_document_yields_the_lifted_fields() {
        // ARRANGE
        let raw = MINIMAL;

        // ACT
        let lifted_version = version(raw).expect("a minimal document must lift");
        let lifted_registry = registry(raw).expect("a minimal document must lift");
        let lifted_extensions = extensions(raw).expect("a minimal document must lift");
        let lifted_secureboot = secureboot(raw).expect("a minimal document must lift");

        // ASSERT
        assert_eq!(lifted_version.as_deref(), Some("v1.2.0"));
        assert_eq!(lifted_registry.as_deref(), Some("ghcr.io/muak-os"));
        assert_eq!(lifted_extensions, ["gpu"]);
        assert_eq!(lifted_secureboot, Some(true));
    }

    #[test]
    fn unknown_keys_and_a_future_api_version_are_ignored() {
        // ARRANGE
        let raw = r#"
            api_version = "muak.dev/config/v9-future"

            [host]
            version = "v2.0.0"
            registry = "ghcr.io/muak-os"
            unknown_field = "ignored"

            [future_section]
            mode = "turbo"
        "#;

        // ACT
        let lifted_version = version(raw).expect("a future document must lift");
        let lifted_registry = registry(raw).expect("a future document must lift");

        // ASSERT
        assert_eq!(lifted_version.as_deref(), Some("v2.0.0"));
        assert_eq!(lifted_registry.as_deref(), Some("ghcr.io/muak-os"));
    }

    #[test]
    fn missing_fields_lift_to_none_and_an_empty_list() {
        // ARRANGE
        let raw = "[host]\nname = \"muak\"\n";

        // ACT
        let lifted_version = version(raw).expect("a missing field is not fatal");
        let lifted_registry = registry(raw).expect("a missing field is not fatal");
        let lifted_secureboot = secureboot(raw).expect("a missing field is not fatal");
        let lifted_extensions = extensions(raw).expect("a missing field is not fatal");

        // ASSERT
        assert_eq!(lifted_version, None);
        assert_eq!(lifted_registry, None);
        assert_eq!(lifted_secureboot, None);
        assert_eq!(lifted_extensions.len(), 0);
    }

    #[test]
    fn empty_text_fields_lift_to_none() {
        // ARRANGE
        let raw = "[host]\nversion = \"\"\nregistry = \"\"\n";

        // ACT
        let lifted_version = version(raw).expect("an empty version is tolerated");
        let lifted_registry = registry(raw).expect("an empty registry is tolerated");

        // ASSERT
        assert_eq!(lifted_version, None);
        assert_eq!(lifted_registry, None);
    }

    #[test]
    fn mistyped_fields_are_rejected_by_name() {
        // ARRANGE
        let raw_version = "[host]\nversion = 3\n";
        let raw_registry = "[host]\nregistry = 3\n";
        let raw_secureboot = "[host]\nsecureboot = \"yes\"\n";
        let raw_extensions = "[host]\nextensions = \"gpu\"\n";
        let raw_element = "[host]\nextensions = [\"gpu\", 3]\n";

        // ACT
        let version_error = version(raw_version).expect_err("a number is not a version");
        let registry_error = registry(raw_registry).expect_err("a number is not a registry");
        let secureboot_error = secureboot(raw_secureboot).expect_err("a string is not a flag");
        let extensions_error = extensions(raw_extensions).expect_err("a string is not a list");
        let element_error = extensions(raw_element).expect_err("a number is not an extension");

        // ASSERT
        assert!(
            version_error.to_string().contains("host.version"),
            "{version_error}"
        );
        assert!(
            version_error.to_string().contains("integer"),
            "{version_error}"
        );
        assert!(
            registry_error.to_string().contains("host.registry"),
            "{registry_error}"
        );
        assert!(
            secureboot_error.to_string().contains("host.secureboot"),
            "{secureboot_error}"
        );
        assert!(
            extensions_error.to_string().contains("host.extensions"),
            "{extensions_error}"
        );
        assert!(
            element_error.to_string().contains("host.extensions"),
            "{element_error}"
        );
    }

    #[test]
    fn invalid_toml_is_a_parse_error() {
        // ARRANGE
        let raw = "this is not a toml document";

        // ACT
        let error = reachability(raw).expect_err("invalid TOML must fail");

        // ASSERT
        assert!(matches!(error, ConfigError::ParseError(_)), "got {error}");
    }

    #[test]
    fn reachability_ignores_unrelated_fields() {
        // ARRANGE
        let previous = "[host]\nport = 50051\nname = \"old\"\nversion = \"v1.0.0\"\n\n\
                        [network]\ndns = [\"1.1.1.1\"]\ninterfaces = [\"eth0\"]\n\n\
                        [vm]\ncpus = 2\n";
        let next = "[host]\nport = 50051\nname = \"new\"\nversion = \"v2.0.0\"\n\n\
                    [network]\ninterfaces = [\"eth0\"]\ndns = [\"1.1.1.1\"]\n\n\
                    [vm]\ncpus = 4\n";

        // ACT
        let previous = reachability(previous).expect("both documents must parse");
        let next = reachability(next).expect("both documents must parse");

        // ASSERT
        assert_eq!(previous, next);
    }

    #[test]
    fn reachability_notices_a_port_change() {
        // ARRANGE
        let previous = "[host]\nport = 50051\n\n[network]\ndns = [\"1.1.1.1\"]\n";
        let next = "[host]\nport = 50052\n\n[network]\ndns = [\"1.1.1.1\"]\n";

        // ACT
        let previous = reachability(previous).expect("both documents must parse");
        let next = reachability(next).expect("both documents must parse");

        // ASSERT
        assert_ne!(previous, next);
    }

    #[test]
    fn reachability_notices_a_network_change() {
        // ARRANGE
        let previous = "[host]\nport = 50051\n\n[network]\ndns = [\"1.1.1.1\"]\n";
        let next = "[host]\nport = 50051\n\n[network]\ndns = [\"9.9.9.9\"]\n";

        // ACT
        let previous = reachability(previous).expect("both documents must parse");
        let next = reachability(next).expect("both documents must parse");

        // ASSERT
        assert_ne!(previous, next);
    }
}
