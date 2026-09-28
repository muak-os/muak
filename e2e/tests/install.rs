mod common;

use core::time::Duration;

use anyhow::{Context as _, Result, ensure};
use common::boot_and_install;
use e2e::artifacts::Artifacts;
use e2e::cli::Cli;
use e2e::vm::TestFixture;
use e2e::{assert_success, assert_success_insecure};
use tempfile::NamedTempFile;
use tokio::time::timeout;

#[cfg(test)]
#[expect(
    clippy::excessive_nesting,
    reason = "closures inside boot_and_install calls"
)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn install() -> Result<()> {
        // ARRANGE
        let artifacts = Artifacts::from_env()?;
        let (fixture, cli) = boot_and_install(&artifacts, |cfg| {
            cfg.host.secureboot = false;
        })
        .await?;

        // ACT
        let disks = assert_success!(cli, ["disks"]).await?;

        // ASSERT
        ensure!(
            disks.contains("nvme0n1"),
            "expected nvme0n1 in disk listing, got: {disks}"
        );
        fixture
            .vm
            .assert_serial_contains("[granola] Running from INSTALLED DISK")?;
        Ok(())
    }

    #[tokio::test]
    async fn install_secureboot() -> Result<()> {
        // ARRANGE
        let artifacts = Artifacts::from_env()?;
        let (fixture, cli) = boot_and_install(&artifacts, |_| {}).await?;

        // ACT
        let security = assert_success!(cli, ["security", "state"]).await?;

        // ASSERT
        ensure!(
            security.contains("Secure Boot: Enabled"),
            "expected Secure Boot to be enabled, got: {security}"
        );
        fixture
            .vm
            .assert_serial_contains("[granola] Running from INSTALLED DISK")?;
        Ok(())
    }

    #[tokio::test]
    async fn install_with_online_generated_config() -> Result<()> {
        // ARRANGE
        let artifacts = Artifacts::from_env()?;
        let fixture = TestFixture::boot_install(&artifacts)?;
        fixture.vm.wait_ready(Duration::from_mins(1)).await?;
        let cli = Cli::new(&artifacts.cli_bin, fixture.vm.host_port)?;

        // ACT
        let raw = cli
            .assert_success_impl(["config", "generate"], true)
            .await?;
        let mut cfg: config::SystemConfig =
            config::parse_from_str(&raw).context("online generate must emit a valid config")?;

        // ASSERT
        ensure!(
            cfg.host.version == common::install_version(),
            "expected version {} prefilled, got: {}",
            common::install_version(),
            cfg.host.version
        );
        ensure!(
            cfg.host.extensions.is_empty(),
            "expected no extensions prefilled, got: {:?}",
            cfg.host.extensions
        );

        "/dev/nvme0n1".clone_into(&mut cfg.disk.system);
        cfg.host.registry = common::install_registry();
        let patched = config::serialize(&cfg).context("serialise generated config")?;
        let config_file = NamedTempFile::new().context("create config tempfile")?;
        std::fs::write(config_file.path(), patched).context("write config tempfile")?;

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
        .map_err(|_elapsed| anyhow::anyhow!("install timed out after 1 minute"))??;

        fixture
            .vm
            .assert_serial_contains("[granola] Running from INSTALLED DISK")?;
        Ok(())
    }
}
