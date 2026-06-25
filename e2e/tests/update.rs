mod common;

use core::time::Duration;

use anyhow::{Result, ensure};
use common::{boot_and_install, install_image};
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
    async fn update() -> Result<()> {
        // ARRANGE
        let artifacts = Artifacts::from_env()?;
        let (fixture, cli) = boot_and_install(&artifacts, |_| {}).await?;

        // ACT
        let stdout = timeout(
            Duration::from_mins(1),
            assert_success!(cli, ["update", "--image", &install_image()]),
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
        Ok(())
    }

    #[tokio::test]
    async fn update_config() -> Result<()> {
        // ARRANGE
        let artifacts = Artifacts::from_env()?;
        let (fixture, cli) = boot_and_install(&artifacts, |_| {}).await?;

        let image = install_image();
        let update_cfg = cli
            .generate_config(|cfg| {
                "/dev/nvme0n1".clone_into(&mut cfg.disk.system);
                cfg.host.image = image;
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
            config_out.contains(&install_image()),
            "expected updated image '{}' in config get output, got: {config_out}",
            install_image()
        );
        fixture.vm.assert_serial_contains("muak.update_id=")?;
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

        let image = install_image();
        let update_cfg = cli
            .generate_config(|cfg| {
                "/dev/nvme0n1".clone_into(&mut cfg.disk.system);
                cfg.host.image = image;
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
            config_out.contains(&install_image()),
            "expected updated image '{}' in config get output, got: {config_out}",
            install_image()
        );
        fixture.vm.assert_serial_contains("muak.update_id=")?;

        let security = assert_success!(cli, ["security", "state"]).await?;
        ensure!(
            security.contains("Secure Boot: Pending (firmware reboot required)"),
            "expected Secure Boot to be enabled, got: {security}"
        );
        Ok(())
    }
}
