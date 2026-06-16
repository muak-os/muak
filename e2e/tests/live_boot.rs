use core::time::Duration;

use anyhow::Result;
use e2e::artifacts::Artifacts;
use e2e::assert_success_insecure;
use e2e::cli::Cli;
use e2e::vm::TestFixture;

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn vm_boots_and_apid_reachable() -> Result<()> {
        // ARRANGE
        let artifacts = Artifacts::from_env()?;
        let fixture = TestFixture::boot_live(&artifacts)?;
        fixture.vm.wait_ready(Duration::from_secs(30)).await?;
        let cli = Cli::new(&artifacts.cli_bin, fixture.vm.host_port)?;

        // ACT & ASSERT
        assert_success_insecure!(cli, ["disks"]).await?;
        Ok(())
    }
}
