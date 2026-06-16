use core::time::Duration;

use anyhow::Result;
use e2e::artifacts::Artifacts;
use e2e::assert_success_insecure;
use e2e::cli::Cli;
use e2e::vm::TestFixture;
use tokio::time::timeout;

pub const DEFAULT_REGISTRY: &str = "ghcr.io/muak-os";
pub const DEFAULT_TAG: &str = "latest";

/// The image used for the initial install.
pub fn install_image() -> String {
    let registry = std::env::var("REGISTRY").unwrap_or_else(|_| DEFAULT_REGISTRY.to_owned());
    let tag = std::env::var("TAG").unwrap_or_else(|_| DEFAULT_TAG.to_owned());
    format!("{registry}/installer:{tag}")
}

/// Boots a VM, runs install with the given extra config patch, waits for the
/// installed system marker, and returns the fixture + CLI driver.
///
/// # Errors
///
/// Returns an error if the VM fails to boot, apid does not become ready, the config cannot be
/// generated, the install command fails, or the installed-boot marker is not found.
pub async fn boot_and_install<F: FnOnce(&mut config::SystemConfig)>(
    artifacts: &Artifacts,
    extra_config: F,
) -> Result<(TestFixture, Cli)> {
    let fixture = TestFixture::boot_install(artifacts)?;

    fixture.vm.wait_ready(Duration::from_mins(1)).await?;

    let cli = Cli::new(&artifacts.cli_bin, fixture.vm.host_port)?;

    let image = install_image();
    let config_file = cli
        .generate_config(|cfg| {
            "/dev/nvme0n1".clone_into(&mut cfg.disk.system);
            cfg.host.image = image;
            extra_config(cfg);
        })
        .await?;

    timeout(
        Duration::from_mins(1),
        assert_success_insecure!(
            cli,
            [
                "install",
                "--config",
                &config_file.path().display().to_string(),
            ]
        ),
    )
    .await
    .map_err(|_elapsed| anyhow::anyhow!("install timed out after 1 minute"))?
    .map_err(|e| anyhow::anyhow!("muakctl install failed: {e}"))?;

    fixture
        .vm
        .assert_serial_contains("[granola] Running from INSTALLED DISK")?;

    Ok((fixture, cli))
}
