mod common;

use anyhow::{Result, ensure};
use common::boot_and_install;
use e2e::artifacts::Artifacts;
use e2e::assert_success;

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
}
