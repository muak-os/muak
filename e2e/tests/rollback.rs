mod common;

use core::time::Duration;

use anyhow::{Result, ensure};
use common::{boot_and_install, install_image};
use e2e::artifacts::Artifacts;
use e2e::assert_success;

#[cfg(test)]
#[expect(
    clippy::excessive_nesting,
    reason = "closures inside boot_and_install calls"
)]
mod tests {
    use super::*;

    /// Triggers an update then abandons the CLI before it can contact provisiond.
    #[expect(
        clippy::integer_division_remainder_used,
        reason = "tokio::select! macro uses % internally"
    )]
    #[tokio::test]
    async fn update_rollback_on_cli_contact_timeout() -> Result<()> {
        // ARRANGE
        let artifacts = Artifacts::from_env()?;
        let (fixture, cli) = boot_and_install(&artifacts, |_| {}).await?;

        let update_cfg = cli
            .generate_config(|cfg| {
                "/dev/nvme0n1".clone_into(&mut cfg.disk.system);
                cfg.host.image = install_image();
            })
            .await?;

        // ACT
        let config_path = update_cfg.path().display().to_string();
        tokio::select! {
            _ = cli.run(
                ["update", "--config", &config_path],
                false,
            ) => {},
            result = fixture.vm.wait_serial_contains(
                "kexec booting into update",
                Duration::from_mins(1),
            ) => {
                result?;
            }
        }

        fixture
            .vm
            .wait_serial_contains("Rebooting for rollback of update", Duration::from_mins(2))
            .await?;

        fixture
            .vm
            .wait_serial_contains(
                "[apid] API daemon ready, listening on",
                Duration::from_mins(1),
            )
            .await?;

        // ASSERT
        let history = assert_success!(cli, ["rollback", "history"]).await?;
        ensure!(
            history.contains("CLI contact check failed"),
            "expected rollback reason in history, got: {history}"
        );

        let config_out = assert_success!(cli, ["config", "get"]).await?;
        ensure!(
            config_out.contains(&install_image()),
            "expected original image '{}' in config after rollback, got: {config_out}",
            install_image()
        );

        fixture.vm.assert_serial_contains("muak.update_id=")?;
        Ok(())
    }
}
