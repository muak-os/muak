//! Post-kexec validation: decide to commit or roll back the update.

use core::time::Duration;
use std::path::Path;

use anyhow::{Context as _, Result, bail};
use tokio::sync::Notify;

use super::commit;
use super::rollback;
use super::snapshot;

static CLI_CONTACT: Notify = Notify::const_new();
const CLI_CONTACT_TIMEOUT: Duration = Duration::from_secs(10);

pub async fn validate(update_id: &str, snapshot_path: &Path) -> Result<()> {
    let old_image = snapshot::read_image(snapshot_path)?;
    let target_image = config::host().image.clone();

    kmsg::info!(
        "Validating update {}: {} -> {}",
        update_id,
        old_image,
        target_image
    );

    if is_old_kernel(update_id) {
        kmsg::warn!("Update {} failed: new kernel did not boot", update_id);
        rollback::apply(
            update_id,
            snapshot_path,
            "Kernel failed to boot (kexec failure)",
        )?;
        return Ok(());
    }

    if let Err(e) = wait_for_cli_contact().await {
        kmsg::warn!("CLI contact check failed for {}: {}", update_id, e);
        rollback::apply(
            update_id,
            snapshot_path,
            &format!("CLI contact check failed: {e}"),
        )?;
        return Ok(());
    }

    if let Err(e) = health_checks() {
        kmsg::warn!("Health checks failed for {}: {}", update_id, e);
        rollback::apply(
            update_id,
            snapshot_path,
            &format!("Health checks failed: {e}"),
        )?;
        return Ok(());
    }

    if let Err(e) = commit::apply().await {
        kmsg::warn!("Commit failed for {}: {}", update_id, e);
        rollback::apply(update_id, snapshot_path, &format!("Commit failed: {e}"))?;
    }

    Ok(())
}

async fn wait_for_cli_contact() -> Result<()> {
    kmsg::info!(
        "Waiting up to {}s for CLI contact",
        CLI_CONTACT_TIMEOUT.as_secs()
    );

    if tokio::time::timeout(CLI_CONTACT_TIMEOUT, CLI_CONTACT.notified())
        .await
        .is_err()
    {
        bail!("no CLI contact within {}s", CLI_CONTACT_TIMEOUT.as_secs());
    }

    kmsg::info!("CLI contact received, proceeding with validation");
    Ok(())
}

/// Wakes the validation task when the CLI contacts provisiond.
pub fn signal_cli_contact() {
    CLI_CONTACT.notify_one();
}

/// Returns true if the current cmdline lacks the update marker (kexec did not boot).
fn is_old_kernel(update_id: &str) -> bool {
    let cmdline = std::fs::read_to_string("/proc/cmdline").unwrap_or_default();
    !cmdline.contains(&format!("muak.update_id={update_id}"))
}

/// Runs all health checks required to declare the update valid.
fn health_checks() -> Result<()> {
    check_state_partition_writable()?;
    check_network_interfaces()?;
    Ok(())
}

/// Checks if the STATE partition is writable.
fn check_state_partition_writable() -> Result<()> {
    let test_path = "/run/state/.update_health_check";
    std::fs::write(test_path, b"ok").context("STATE partition not writable")?;
    std::fs::remove_file(test_path).context("Failed to clean up health check file")?;
    Ok(())
}

/// Checks if at least one non-loopback network interface is available.
fn check_network_interfaces() -> Result<()> {
    let net_dir =
        std::fs::read_dir("/sys/class/net").context("Failed to read network interfaces")?;

    let non_loopback_count = net_dir
        .filter_map(core::result::Result::ok)
        .filter(|entry| entry.file_name() != "lo")
        .count();

    if non_loopback_count == 0 {
        bail!("No non-loopback network interfaces found");
    }

    Ok(())
}
