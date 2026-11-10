mod common;

use core::time::Duration;

use anyhow::{Result, ensure};
use common::{boot_and_install, install_registry, install_version, system_disk};
use e2e::artifacts::Artifacts;
use e2e::assert_success;
use tokio::time::timeout;

#[cfg(test)]
#[expect(
    clippy::excessive_nesting,
    reason = "closures inside boot_and_install calls"
)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn update_explicit_version() -> Result<()> {
        // ARRANGE
        let artifacts = Artifacts::from_env()?;
        let (fixture, cli) = boot_and_install(&artifacts, |_| {}).await?;

        // ACT
        let stdout = timeout(
            Duration::from_mins(1),
            assert_success!(cli, ["update", "--version", &install_version(), "--force"]),
        )
        .await
        .map_err(|_elapsed| {
            let serial = fixture.vm.read_serial().unwrap_or_default();
            let stderr = fixture.vm.read_stderr().unwrap_or_default();
            anyhow::anyhow!(
                "update timed out\
                 \n\n--- serial log ---\n{serial}\
                 \n\n--- stderr ---\n{stderr}"
            )
        })?
        .map_err(|e| anyhow::anyhow!("muakctl update failed: {e}"))?;

        // ASSERT
        ensure!(
            stdout.contains("committed successfully"),
            "expected 'committed successfully' in update output, got: {stdout}"
        );
        fixture.vm.assert_serial_contains("muak.update_id=")?;
        fixture
            .vm
            .assert_serial_contains("Update committed to the boot partition")?;
        Ok(())
    }

    #[tokio::test]
    async fn update_skips_when_already_current() -> Result<()> {
        // ARRANGE
        let artifacts = Artifacts::from_env()?;
        let (fixture, cli) = boot_and_install(&artifacts, |_| {}).await?;

        // ACT
        let stdout = timeout(
            Duration::from_mins(1),
            assert_success!(cli, ["update", "--version", &install_version()]),
        )
        .await
        .map_err(|_elapsed| {
            let serial = fixture.vm.read_serial().unwrap_or_default();
            let stderr = fixture.vm.read_stderr().unwrap_or_default();
            anyhow::anyhow!(
                "update timed out\
                 \n\n--- serial log ---\n{serial}\
                 \n\n--- stderr ---\n{stderr}"
            )
        })?
        .map_err(|e| anyhow::anyhow!("muakctl update failed: {e}"))?;

        // ASSERT
        ensure!(
            stdout.contains("nothing to do"),
            "expected 'nothing to do' in update output, got: {stdout}"
        );
        let serial = fixture.vm.read_serial()?;
        ensure!(
            !serial.contains("muak.update_id="),
            "no update must be staged, but the serial log shows one"
        );
        Ok(())
    }

    #[tokio::test]
    async fn update_config() -> Result<()> {
        // ARRANGE
        let artifacts = Artifacts::from_env()?;
        let (fixture, cli) = boot_and_install(&artifacts, |_| {}).await?;

        let (registry, version) = (install_registry(), install_version());
        let update_cfg = cli
            .generate_config(|cfg| {
                cfg.disks = vec![system_disk()];
                cfg.host.registry = registry;
                cfg.host.version = version;
            })
            .await?;

        // ACT
        let stdout = timeout(
            Duration::from_mins(1),
            assert_success!(
                cli,
                [
                    "update",
                    "--config",
                    &update_cfg.path().display().to_string(),
                    "--force",
                ]
            ),
        )
        .await
        .map_err(|_elapsed| {
            let serial = fixture.vm.read_serial().unwrap_or_default();
            let stderr = fixture.vm.read_stderr().unwrap_or_default();
            anyhow::anyhow!(
                "update --config timed out\
                 \n\n--- serial log ---\n{serial}\
                 \n\n--- stderr ---\n{stderr}"
            )
        })?
        .map_err(|e| anyhow::anyhow!("muakctl update --config failed: {e}"))?;

        // ASSERT
        ensure!(
            stdout.contains("committed successfully"),
            "expected 'committed successfully' in update --config output, got: {stdout}"
        );
        let config_out = assert_success!(cli, ["config", "get"]).await?;
        ensure!(
            config_out.contains(&install_version()),
            "expected updated version '{}' in config get output, got: {config_out}",
            install_version()
        );
        fixture.vm.assert_serial_contains("muak.update_id=")?;
        fixture
            .vm
            .assert_serial_contains("Update committed to the boot partition")?;
        Ok(())
    }

    #[tokio::test]
    async fn update_config_secureboot() -> Result<()> {
        // ARRANGE
        let artifacts = Artifacts::from_env()?;
        let (fixture, cli) = boot_and_install(&artifacts, |cfg| {
            cfg.host.secureboot = false;
        })
        .await?;

        let (registry, version) = (install_registry(), install_version());
        let update_cfg = cli
            .generate_config(|cfg| {
                cfg.disks = vec![system_disk()];
                cfg.host.registry = registry;
                cfg.host.version = version;
                cfg.host.secureboot = true;
            })
            .await?;

        // ACT
        let stdout = timeout(
            Duration::from_mins(1),
            assert_success!(
                cli,
                [
                    "update",
                    "--config",
                    &update_cfg.path().display().to_string(),
                ]
            ),
        )
        .await
        .map_err(|_elapsed| {
            let serial = fixture.vm.read_serial().unwrap_or_default();
            let stderr = fixture.vm.read_stderr().unwrap_or_default();
            anyhow::anyhow!(
                "update --config timed out\
                 \n\n--- serial log ---\n{serial}\
                 \n\n--- stderr ---\n{stderr}"
            )
        })?
        .map_err(|e| anyhow::anyhow!("muakctl update --config failed: {e}"))?;

        // ASSERT
        ensure!(
            stdout.contains("committed successfully"),
            "expected 'committed successfully' in update --config output, got: {stdout}"
        );
        let config_out = assert_success!(cli, ["config", "get"]).await?;
        ensure!(
            config_out.contains(&install_version()),
            "expected updated version '{}' in config get output, got: {config_out}",
            install_version()
        );
        fixture.vm.assert_serial_contains("muak.update_id=")?;
        fixture
            .vm
            .assert_serial_contains("Update committed to the boot partition")?;

        let security = assert_success!(cli, ["security", "state"]).await?;
        ensure!(
            security.contains("Secure Boot: Pending (firmware reboot required)"),
            "expected Secure Boot to be enabled, got: {security}"
        );
        Ok(())
    }

    #[tokio::test]
    async fn update_config_secureboot_disable() -> Result<()> {
        // ARRANGE
        let artifacts = Artifacts::from_env()?;
        let (fixture, cli) = boot_and_install(&artifacts, |_| {}).await?;

        let (registry, version) = (install_registry(), install_version());
        let update_cfg = cli
            .generate_config(|cfg| {
                cfg.disks = vec![system_disk()];
                cfg.host.registry = registry;
                cfg.host.version = version;
                cfg.host.secureboot = false;
            })
            .await?;

        // ACT
        let stdout = timeout(
            Duration::from_mins(1),
            assert_success!(
                cli,
                [
                    "update",
                    "--config",
                    &update_cfg.path().display().to_string(),
                ]
            ),
        )
        .await
        .map_err(|_elapsed| {
            let serial = fixture.vm.read_serial().unwrap_or_default();
            let stderr = fixture.vm.read_stderr().unwrap_or_default();
            anyhow::anyhow!(
                "update --config timed out\
                 \n\n--- serial log ---\n{serial}\
                 \n\n--- stderr ---\n{stderr}"
            )
        })?
        .map_err(|e| anyhow::anyhow!("muakctl update --config failed: {e}"))?;

        // ASSERT
        ensure!(
            stdout.contains("committed successfully"),
            "expected 'committed successfully' in update --config output, got: {stdout}"
        );
        let config_out = assert_success!(cli, ["config", "get"]).await?;
        ensure!(
            config_out.contains("secureboot = false"),
            "expected secureboot to be disabled, got: {config_out}"
        );
        fixture.vm.assert_serial_contains("muak.update_id=")?;
        fixture
            .vm
            .assert_serial_contains("Update committed to the boot partition")?;
        Ok(())
    }
}
