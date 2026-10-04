//! Default config template generation, prefilled from the booted medium.

use anyhow::{Context as _, Result};
use config::SystemConfig;
use tokio::task;

use crate::medium;

/// Renders the default config with the medium's release and extension selection prefilled.
///
/// # Errors
///
/// Returns an error when the embedded default config fails to parse or serialize.
pub fn render(version: String, extensions: &[String]) -> Result<String> {
    let mut config: SystemConfig = config::parse_from_str(&config::serialize_default())
        .context("embedded default config is invalid")?;
    config.host.version = version;
    config.host.extensions = sorted(extensions);

    Ok(config::serialize(&config)?)
}

/// Generates the default config from the booted medium's metadata.
///
/// # Errors
///
/// Returns an error when the medium carries no version metadata.
pub async fn generate() -> Result<String> {
    let (version, extensions) =
        task::spawn_blocking(|| -> anyhow::Result<(String, Vec<String>)> {
            let version = medium::version()?;
            let extensions = medium::profile()
                .map(|booted| booted.customization().extensions().to_vec())
                .unwrap_or_default();

            Ok((version, extensions))
        })
        .await
        .context("template generation task failed")??;

    render(version, &extensions)
}

fn sorted(extensions: &[String]) -> Vec<String> {
    let mut names = extensions.to_vec();
    names.sort_unstable();
    names.dedup();

    names
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn render_prefills_version_and_sorted_extensions() {
        // ARRANGE / ACT
        let rendered = render(
            "v1.2.3".to_owned(),
            &["muak-os/virtio".to_owned(), "muak-os/qemu".to_owned()],
        )
        .expect("render");

        // ASSERT
        let config: SystemConfig = config::parse_from_str(&rendered).expect("valid config");
        assert_eq!(config.host.version, "v1.2.3");
        assert_eq!(config.host.extensions, ["muak-os/qemu", "muak-os/virtio"]);
    }

    #[test]
    fn render_dedupes_repeated_extensions() {
        // ARRANGE / ACT
        let rendered = render(
            "v1.2.3".to_owned(),
            &[
                "muak-os/qemu".to_owned(),
                "muak-os/qemu".to_owned(),
                "muak-os/virtio".to_owned(),
            ],
        )
        .expect("render");

        // ASSERT
        let config: SystemConfig = config::parse_from_str(&rendered).expect("valid config");
        assert_eq!(config.host.extensions, ["muak-os/qemu", "muak-os/virtio"]);
    }
}
