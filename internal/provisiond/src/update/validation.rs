//! Post-kexec validation: decide to commit or roll back the update.

use anyhow::{Context, Result, bail};

use super::commit;
use super::marker::ValidationMarker;
use super::rollback;

/// Decides whether to commit or roll back a pending update.
pub async fn check_pending(m: &ValidationMarker) -> Result<()> {
    println!(
        "Found pending validation for update {} -> {}",
        m.current_image, m.target_image
    );

    if is_old_kernel(m) {
        println!(
            "Update {} failed - new kernel did not boot successfully",
            &m.update_id
        );
        rollback::rollback(m, "Kernel failed to boot (kexec failure)")?;
    } else if let Err(e) = health_checks() {
        println!("Health checks failed: {}", e);
        rollback::rollback(m, &format!("Health checks failed: {}", e))?;
    } else {
        commit::commit(m).await?;
    }

    Ok(())
}

/// Returns true if the current cmdline lacks the update marker (kexec did not boot).
fn is_old_kernel(m: &ValidationMarker) -> bool {
    let cmdline = std::fs::read_to_string("/proc/cmdline").unwrap_or_default();
    !cmdline.contains(&format!("muak.update_id={}", m.update_id))
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
        .filter_map(|e| e.ok())
        .filter(|e| e.file_name() != "lo")
        .count();

    if non_loopback_count == 0 {
        bail!("No non-loopback network interfaces found");
    }

    Ok(())
}
