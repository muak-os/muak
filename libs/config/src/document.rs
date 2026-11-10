//! Schema-tolerant lifting of individual fields from a config document.

use crate::error::{ConfigError, Result};

/// `network` subtree and `host.port`, compared by their canonical rendering.
#[derive(Debug, PartialEq, Eq)]
pub struct Reachability {
    network: Option<String>,
    port: Option<String>,
}

/// Subset of host fields an update build needs from a config document.
#[derive(Debug, PartialEq, Eq)]
pub struct HostSpec {
    /// Registry serving the catalog.
    pub registry: String,
    /// Resolved target catalog release.
    pub version: String,
    /// Requested system extension images.
    pub extensions: Vec<String>,
    /// Whether the build must ship a Secure Boot signed UKI.
    pub secureboot: bool,
}

/// Projects the update build inputs from `raw`, resolving the target version.
///
/// # Errors
///
/// Returns [`ConfigError::ParseError`] when the document is not TOML,
/// [`ConfigError::ValidationError`] when `host.registry` is absent or empty or a
/// lifted field is mistyped, and when no single target version can be resolved.
pub fn host_spec(raw: &str, requested_version: Option<&str>) -> Result<HostSpec> {
    let declared = version(raw)?;

    Ok(HostSpec {
        registry: registry(raw)?.ok_or_else(|| {
            ConfigError::ValidationError("host.registry must be specified".to_owned())
        })?,
        version: target_version(declared.as_deref(), requested_version)?,
        extensions: extensions(raw)?,
        secureboot: secureboot(raw)?.unwrap_or(false),
    })
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

/// Reports whether two documents differ in reachability-relevant fields.
///
/// # Errors
///
/// Returns [`ConfigError::ParseError`] when either document is not TOML.
pub fn isolates(previous: &str, next: &str) -> Result<bool> {
    Ok(reachability(previous)? != reachability(next)?)
}

fn target_version(document: Option<&str>, requested: Option<&str>) -> Result<String> {
    match (document, requested) {
        (Some(document), Some(requested)) if document != requested => {
            Err(ConfigError::ValidationError(format!(
                "version '{requested}' does not match host.version '{document}' in the config"
            )))
        }
        (_, Some(requested)) => Ok(requested.to_owned()),
        (Some(document), None) => Ok(document.to_owned()),
        (None, None) => Err(ConfigError::ValidationError(
            "host.version must be specified in the config or with --version".to_owned(),
        )),
    }
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
    use super::{
        HostSpec, extensions, host_spec, isolates, reachability, registry, secureboot, version,
    };
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
    fn a_minimal_document_projects_the_host_spec() {
        // ARRANGE
        let raw = MINIMAL;

        // ACT
        let spec = host_spec(raw, None).expect("a minimal document must project");

        // ASSERT
        assert_eq!(
            spec,
            HostSpec {
                registry: "ghcr.io/muak-os".to_owned(),
                version: "v1.2.0".to_owned(),
                extensions: vec!["gpu".to_owned()],
                secureboot: true,
            }
        );
    }

    #[test]
    fn a_requested_version_fills_a_document_without_one() {
        // ARRANGE
        let raw = "[host]\nregistry = \"ghcr.io/muak-os\"\nversion = \"\"\n";

        // ACT
        let spec = host_spec(raw, Some("v2.0.0")).expect("the request version must fill in");

        // ASSERT
        assert_eq!(spec.version, "v2.0.0");
    }

    #[test]
    fn a_requested_version_matching_the_document_is_accepted() {
        // ARRANGE
        let raw = "[host]\nregistry = \"ghcr.io/muak-os\"\nversion = \"v2.0.0\"\n";

        // ACT
        let spec = host_spec(raw, Some("v2.0.0")).expect("a matching version must be accepted");

        // ASSERT
        assert_eq!(spec.version, "v2.0.0");
    }

    #[test]
    fn disagreeing_versions_are_rejected_naming_both() {
        // ARRANGE
        let raw = "[host]\nregistry = \"ghcr.io/muak-os\"\nversion = \"v2.0.0\"\n";

        // ACT
        let error =
            host_spec(raw, Some("v2.1.0")).expect_err("a disagreeing version must be rejected");

        // ASSERT
        assert!(error.to_string().contains("v2.0.0"), "{error}");
        assert!(error.to_string().contains("v2.1.0"), "{error}");
    }

    #[test]
    fn a_version_from_neither_source_is_rejected() {
        // ARRANGE
        let raw = "[host]\nregistry = \"ghcr.io/muak-os\"\n";

        // ACT
        let error = host_spec(raw, None).expect_err("a missing version must be rejected");

        // ASSERT
        let message = error.to_string();
        assert!(message.contains("host.version"), "{message}");
        assert!(message.contains("--version"), "{message}");
    }

    #[test]
    fn missing_or_empty_registry_is_rejected() {
        // ARRANGE
        let missing = "[host]\nversion = \"v1.2.0\"\n";
        let empty = "[host]\nregistry = \"\"\nversion = \"v1.2.0\"\n";

        // ACT
        let missing_error = host_spec(missing, None).expect_err("missing registry must fail");
        let empty_error = host_spec(empty, None).expect_err("empty registry must fail");

        // ASSERT
        assert!(
            missing_error.to_string().contains("host.registry"),
            "{missing_error}"
        );
        assert!(
            empty_error.to_string().contains("host.registry"),
            "{empty_error}"
        );
    }

    #[test]
    fn a_future_document_projects_the_host_spec() {
        // ARRANGE
        let raw = r#"
            api_version = "muak.dev/config/v9-future"

            [host]
            registry = "ghcr.io/muak-os"
            version = "v1.2.0"
            extensions = ["gpu", "debug"]
            secureboot = true
            unknown_field = "ignored"

            [future_section]
            mode = "turbo"
        "#;

        // ACT
        let spec = host_spec(raw, None).expect("a future document must still project");

        // ASSERT
        assert_eq!(spec.version, "v1.2.0");
        assert_eq!(spec.extensions, ["gpu", "debug"]);
        assert!(spec.secureboot);
    }

    #[test]
    fn a_non_bool_secureboot_is_rejected() {
        // ARRANGE
        let raw =
            "[host]\nregistry = \"ghcr.io/muak-os\"\nversion = \"v1.2.0\"\nsecureboot = \"yes\"\n";

        // ACT
        let error = host_spec(raw, None).expect_err("non bool secureboot must fail");

        // ASSERT
        assert!(error.to_string().contains("host.secureboot"), "{error}");
    }

    #[test]
    fn an_absent_secureboot_defaults_to_false() {
        // ARRANGE
        let raw = "[host]\nregistry = \"ghcr.io/muak-os\"\nversion = \"v1.2.0\"\n";

        // ACT
        let spec = host_spec(raw, None).expect("an absent flag is not fatal");

        // ASSERT
        assert!(!spec.secureboot);
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

    #[test]
    fn a_network_change_isolates() {
        // ARRANGE
        let previous = "[host]\nport = 50051\n\n[network]\ndns = [\"1.1.1.1\"]\n";
        let next = "[host]\nport = 50051\n\n[network]\ndns = [\"9.9.9.9\"]\n";

        // ACT
        let isolating = isolates(previous, next).expect("both documents must parse");

        // ASSERT
        assert!(isolating, "a network change must isolate");
    }

    #[test]
    fn a_port_change_isolates() {
        // ARRANGE
        let previous = "[host]\nport = 50051\n\n[network]\ndns = [\"1.1.1.1\"]\n";
        let next = "[host]\nport = 50052\n\n[network]\ndns = [\"1.1.1.1\"]\n";

        // ACT
        let isolating = isolates(previous, next).expect("both documents must parse");

        // ASSERT
        assert!(isolating, "a port change must isolate");
    }

    #[test]
    fn unrelated_field_changes_do_not_isolate() {
        // ARRANGE
        let previous = "[host]\nport = 50051\nname = \"old\"\nversion = \"v1.0.0\"\n\n\
                        [network]\ndns = [\"1.1.1.1\"]\ninterfaces = [\"eth0\"]\n\n\
                        [vm]\ncpus = 2\n\n\
                        [[disks]]\ndevice = \"/dev/vda\"\n";
        let next = "[host]\nport = 50051\nname = \"new\"\nversion = \"v2.0.0\"\n\n\
                    [network]\ninterfaces = [\"eth0\"]\ndns = [\"1.1.1.1\"]\n\n\
                    [vm]\ncpus = 4\n\n\
                    [[disks]]\ndevice = \"/dev/vdb\"\n";

        // ACT
        let isolating = isolates(previous, next).expect("both documents must parse");

        // ASSERT
        assert!(!isolating, "unrelated fields must not isolate");
    }

    #[test]
    fn identical_documents_do_not_isolate() {
        // ARRANGE
        let raw = "[host]\nport = 50051\n\n[network]\ndns = [\"1.1.1.1\"]\n";

        // ACT
        let isolating = isolates(raw, raw).expect("both documents must parse");

        // ASSERT
        assert!(!isolating, "an identical document must not isolate");
    }

    #[test]
    fn an_invalid_document_fails_to_compare() {
        // ARRANGE
        let raw = "this is not a toml document";
        let valid = "[host]\nport = 50051\n";

        // ACT
        let previous_error = isolates(raw, valid).expect_err("invalid TOML must fail");
        let next_error = isolates(valid, raw).expect_err("invalid TOML must fail");

        // ASSERT
        assert!(
            matches!(previous_error, ConfigError::ParseError(_)),
            "got {previous_error}"
        );
        assert!(
            matches!(next_error, ConfigError::ParseError(_)),
            "got {next_error}"
        );
    }
}
