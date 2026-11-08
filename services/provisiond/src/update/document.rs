//! Lenient extraction of build inputs from an operator-supplied config document.

use anyhow::{Context as _, Result, bail};

/// Scalar inputs an update build needs from the operator-supplied document.
#[derive(Debug)]
pub(crate) struct BuildInputs {
    /// Registry serving the catalog.
    pub registry: String,
    /// Target catalog release.
    pub version: String,
    /// Requested system extension images.
    pub extensions: Vec<String>,
    /// Whether the update must ship a Secure Boot signed UKI.
    pub secureboot: bool,
}

/// Extracts the build inputs from a config document without validating it.
///
/// # Errors
///
/// Returns an error when the document is not TOML or a lifted field is mistyped.
pub(crate) fn build_inputs(raw: &str) -> Result<BuildInputs> {
    let document: toml::Value = toml::from_str(raw).context("Invalid config document")?;
    let host = document.get("host");

    Ok(BuildInputs {
        registry: scalar(host, "registry")?,
        version: scalar(host, "version")?,
        extensions: extensions(host),
        secureboot: boolean(host, "secureboot")?,
    })
}

fn scalar(host: Option<&toml::Value>, field: &str) -> Result<String> {
    let text = host
        .and_then(|host| host.get(field))
        .with_context(|| format!("host.{field} must be specified"))?
        .as_str()
        .with_context(|| format!("host.{field} must be a string"))?;

    if text.is_empty() {
        bail!("host.{field} must be specified");
    }

    Ok(text.to_owned())
}

fn boolean(host: Option<&toml::Value>, field: &str) -> Result<bool> {
    match host.and_then(|host| host.get(field)) {
        None => Ok(false),
        Some(value) => value
            .as_bool()
            .with_context(|| format!("host.{field} must be a boolean")),
    }
}

fn extensions(host: Option<&toml::Value>) -> Vec<String> {
    host.and_then(|host| host.get("extensions"))
        .and_then(|value| value.as_array())
        .map_or_default(|items| {
            items
                .iter()
                .filter_map(|item| item.as_str().map(str::to_owned))
                .collect()
        })
}

#[cfg(test)]
mod tests {
    use super::build_inputs;

    #[test]
    fn minimal_document_yields_the_build_inputs() {
        // ARRANGE
        let raw = r#"
            [host]
            registry = "ghcr.io/muak-os"
            version = "v1.2.0"
        "#;

        // ACT
        let inputs = build_inputs(raw).expect("minimal document must parse");

        // ASSERT
        assert_eq!(inputs.registry, "ghcr.io/muak-os");
        assert_eq!(inputs.version, "v1.2.0");
        assert_eq!(inputs.extensions.len(), 0);
        assert!(!inputs.secureboot);
    }

    #[test]
    fn unknown_keys_and_future_api_version_are_ignored() {
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
        let inputs = build_inputs(raw).expect("future document must still parse");

        // ASSERT
        assert_eq!(inputs.version, "v1.2.0");
        assert_eq!(inputs.extensions, ["gpu", "debug"]);
        assert!(inputs.secureboot);
    }

    #[test]
    fn missing_registry_is_rejected() {
        // ARRANGE
        let raw = r#"
            [host]
            version = "v1.2.0"
        "#;

        // ACT
        let error = build_inputs(raw).expect_err("missing registry must fail");

        // ASSERT
        assert!(error.to_string().contains("host.registry"), "got {error}");
    }

    #[test]
    fn empty_registry_is_rejected() {
        // ARRANGE
        let raw = r#"
            [host]
            registry = ""
            version = "v1.2.0"
        "#;

        // ACT
        let error = build_inputs(raw).expect_err("empty registry must fail");

        // ASSERT
        assert!(error.to_string().contains("host.registry"), "got {error}");
    }

    #[test]
    fn missing_version_is_rejected() {
        // ARRANGE
        let raw = r#"
            [host]
            registry = "ghcr.io/muak-os"
        "#;

        // ACT
        let error = build_inputs(raw).expect_err("missing version must fail");

        // ASSERT
        assert!(error.to_string().contains("host.version"), "got {error}");
    }

    #[test]
    fn non_string_version_is_rejected() {
        // ARRANGE
        let raw = r#"
            [host]
            registry = "ghcr.io/muak-os"
            version = 12
        "#;

        // ACT
        let error = build_inputs(raw).expect_err("non string version must fail");

        // ASSERT
        assert!(error.to_string().contains("host.version"), "got {error}");
    }

    #[test]
    fn absent_extensions_yield_an_empty_list() {
        // ARRANGE
        let raw = r#"
            [host]
            registry = "ghcr.io/muak-os"
            version = "v1.2.0"
        "#;

        // ACT
        let inputs = build_inputs(raw).expect("document without extensions must parse");

        // ASSERT
        assert_eq!(inputs.extensions.len(), 0);
    }

    #[test]
    fn non_bool_secureboot_is_rejected() {
        // ARRANGE
        let raw = r#"
            [host]
            registry = "ghcr.io/muak-os"
            version = "v1.2.0"
            secureboot = "yes"
        "#;

        // ACT
        let error = build_inputs(raw).expect_err("non bool secureboot must fail");

        // ASSERT
        assert!(error.to_string().contains("host.secureboot"), "got {error}");
    }

    #[test]
    fn secureboot_flag_round_trips() {
        // ARRANGE
        let document = |secureboot: bool| {
            format!(
                "[host]\nregistry = \"ghcr.io/muak-os\"\nversion = \"v1.2.0\"\nsecureboot = {secureboot}\n"
            )
        };

        // ACT
        let enabled = build_inputs(&document(true)).expect("enabled document must parse");
        let disabled = build_inputs(&document(false)).expect("disabled document must parse");

        // ASSERT
        assert!(enabled.secureboot);
        assert!(!disabled.secureboot);
    }
}
