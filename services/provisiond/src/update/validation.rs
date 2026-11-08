//! Post-kexec validation: decide to commit or roll back the update.

use core::time::Duration;
use std::path::Path;

use anyhow::{Context as _, Result, bail};
use tokio::sync::Notify;

use super::{UPDATE_DIR, commit, readiness, rollback, snapshot};
use crate::medium;

static CLI_CONTACT: Notify = Notify::const_new();
const CONFIRMATION_WINDOW: Duration = Duration::from_mins(5);

pub async fn validate(update_id: &str, snapshot_path: &Path) -> Result<()> {
    let previous = std::fs::read_to_string(snapshot_path).ok();
    let old_version = previous
        .as_deref()
        .and_then(snapshot::host_version)
        .unwrap_or_else(|| "unknown".to_owned());
    let target_version = medium::version().unwrap_or_else(|error| {
        kmsg::warn!("Failed to read the booted artifact version: {error:#}");
        "unknown".to_owned()
    });

    kmsg::info!(
        "Validating update {}: {} -> {}",
        update_id,
        old_version,
        target_version
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

    if let Err(e) = readiness::await_all_ready().await {
        kmsg::warn!("Services not ready for {}: {:#}", update_id, e);
        rollback::apply(
            update_id,
            snapshot_path,
            &format!("Services not ready: {e:#}"),
        )?;
        return Ok(());
    }
    kmsg::info!("All supervised services ready for {}", update_id);

    if isolating_change(previous.as_deref(), config::system::config()?) {
        kmsg::info!(
            "Update {} changed reachability config, waiting for CLI confirmation",
            update_id
        );
        if let Err(e) = wait_for_confirmation().await {
            kmsg::warn!("Operator confirmation failed for {}: {:#}", update_id, e);
            rollback::apply(
                update_id,
                snapshot_path,
                &format!("Operator confirmation failed: {e:#}"),
            )?;
            return Ok(());
        }
    }

    if let Err(e) = health_checks() {
        kmsg::warn!("Health checks failed for {}: {:#}", update_id, e);
        rollback::apply(
            update_id,
            snapshot_path,
            &format!("Health checks failed: {e:#}"),
        )?;
        return Ok(());
    }

    if let Err(e) = commit::apply().await {
        kmsg::warn!("Commit failed for {}: {:#}", update_id, e);
        rollback::apply(update_id, snapshot_path, &format!("Commit failed: {e:#}"))?;
    }

    Ok(())
}

async fn wait_for_confirmation() -> Result<()> {
    kmsg::info!(
        "Waiting up to {}s for operator confirmation",
        CONFIRMATION_WINDOW.as_secs()
    );

    if tokio::time::timeout(CONFIRMATION_WINDOW, CLI_CONTACT.notified())
        .await
        .is_err()
    {
        bail!(
            "no operator contact within {}s",
            CONFIRMATION_WINDOW.as_secs()
        );
    }

    kmsg::info!("Operator confirmed, proceeding with validation");

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

fn isolating_change(previous: Option<&str>, next: &config::system::Config) -> bool {
    let Some(previous) = previous else {
        kmsg::warn!("Previous config is unreadable; treating the update as isolating");
        return true;
    };

    let next = match config::system::serialize(next) {
        Ok(serialized) => serialized,
        Err(error) => {
            kmsg::warn!("Next config could not be serialized ({error:#}); skipping isolation");
            return false;
        }
    };

    let (Some(previous), Some(next)) = (reachability(previous), reachability(&next)) else {
        kmsg::warn!("A config document is not valid TOML; skipping isolation detection");
        return false;
    };

    previous != next
}

fn reachability(raw: &str) -> Option<toml::Value> {
    let document: toml::Value = toml::from_str(raw).ok()?;
    let mut lifted = toml::value::Table::new();

    if let Some(network) = document.get("network") {
        lifted.insert("network".to_owned(), network.clone());
    }
    if let Some(port) = document.get("host").and_then(|host| host.get("port")) {
        lifted.insert("port".to_owned(), port.clone());
    }

    Some(toml::Value::Table(lifted))
}

/// Runs all health checks required to declare the update valid.
fn health_checks() -> Result<()> {
    check_state_partition_writable()?;
    check_network_interfaces()?;

    Ok(())
}

/// Checks if the STATE partition is writable.
fn check_state_partition_writable() -> Result<()> {
    let test_path = Path::new(UPDATE_DIR).join(".health_check");
    std::fs::write(&test_path, b"ok").context("STATE partition not writable")?;
    std::fs::remove_file(&test_path).context("Failed to clean up health check file")?;

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

#[cfg(test)]
mod tests {
    use super::isolating_change;

    #[test]
    fn unparseable_previous_config_is_not_treated_as_isolating() {
        // ARRANGE
        let next = config::system::Config::default();
        let previous = "this is not a toml document";

        // ACT
        let isolating = isolating_change(Some(previous), &next);

        // ASSERT
        assert!(!isolating, "an unparseable document cannot be compared");
    }

    #[test]
    fn unreadable_previous_config_is_treated_as_isolating() {
        // ARRANGE
        let next = config::system::Config::default();

        // ACT
        let isolating = isolating_change(None, &next);

        // ASSERT
        assert!(isolating);
    }

    #[test]
    fn unchanged_reachability_is_not_isolating() {
        // ARRANGE
        let next = config::system::Config::default();
        let previous = config::system::serialize(&next).expect("default config serializes");

        // ACT
        let isolating = isolating_change(Some(&previous), &next);

        // ASSERT
        assert!(
            !isolating,
            "an identical reachability config must not isolate"
        );
    }

    #[test]
    fn api_port_change_of_a_future_schema_isolates() {
        // ARRANGE
        let next = config::system::Config::default();
        let previous = format!(
            "api_version = \"muak.dev/config/v9-future\"\n\n[host]\nport = {}\n",
            next.host.port + 1
        );

        // ACT
        let isolating = isolating_change(Some(&previous), &next);

        // ASSERT
        assert!(isolating, "a port change must require confirmation");
    }

    #[test]
    fn network_change_of_a_future_schema_isolates() {
        // ARRANGE
        let next = config::system::Config::default();
        let previous = format!(
            "api_version = \"muak.dev/config/v9-future\"\n\n[host]\nport = {}\n\n\
             [network]\nipv6 = {}\n",
            next.host.port, !next.network.ipv6
        );

        // ACT
        let isolating = isolating_change(Some(&previous), &next);

        // ASSERT
        assert!(isolating, "a network change must require confirmation");
    }
}
