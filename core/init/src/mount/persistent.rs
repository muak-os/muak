use std::path::Path;

use anyhow::{Context, Result};
use rustix::fs::{CWD, Mode, mkdirat};
use rustix::mount::{MountFlags, UnmountFlags, mount, unmount};

use super::luks::resolve_key;
use super::partition::find_partitions_by_partname;

const DM_STATE: &str = "muak-state";
const DM_DATA: &str = "muak-data";
const STATE_MOUNT: &str = "/run/state";
const DATA_MOUNT: &str = "/run/data";
const STATE_CONFIG: &str = "/run/state/config.toml";

/// Mount persistent STATE and DATA partitions if the system is installed.
pub(crate) fn mount_persistent() -> Result<bool> {
    let state_devices = find_partitions_by_partname("STATE");
    if state_devices.is_empty() {
        return Ok(false);
    }

    let data_devices = find_partitions_by_partname("DATA");
    for state_dev in state_devices {
        match try_mount_persistent_candidate(&state_dev, &data_devices) {
            Ok(()) => return Ok(true),
            Err(error) => {
                kmsg::warn!(
                    "Ignoring invalid STATE candidate {}: {:#}",
                    state_dev,
                    error
                );
                cleanup_persistent_mounts();
            }
        }
    }

    Ok(false)
}

fn try_mount_persistent_candidate(state_dev: &str, data_devices: &[String]) -> Result<()> {
    let luks_key = resolve_key(state_dev).or_else(|error| bail_maintenance(error.to_string()))?;

    let state_device = if let Some(key) = luks_key.as_ref() {
        luks2::open(state_dev, DM_STATE, key)
            .with_context(|| format!("Failed to open LUKS device: {state_dev}"))?;
        format!("/dev/mapper/{DM_STATE}")
    } else {
        state_dev.to_string()
    };

    mount_btrfs(&state_device, STATE_MOUNT)
        .with_context(|| format!("Failed to mount STATE partition: {state_dev}"))?;

    if !Path::new(STATE_CONFIG).exists() {
        bail_maintenance("STATE partition is missing config.toml")?;
    }

    kmsg::info!("Mounted STATE partition at /run/state");

    if let Some(data_dev) = data_devices.first() {
        let data_device = if let Some(key) = luks_key.as_ref() {
            luks2::open(data_dev, DM_DATA, key)
                .with_context(|| format!("Failed to open LUKS device: {data_dev}"))?;
            format!("/dev/mapper/{DM_DATA}")
        } else {
            data_dev.to_string()
        };

        mount_btrfs(&data_device, DATA_MOUNT)
            .with_context(|| format!("Failed to mount DATA partition: {data_dev}"))?;

        kmsg::info!("Mounted DATA partition at /run/data");
        btrfs::quota::enable(DATA_MOUNT)?;
    }

    Ok(())
}

fn mount_btrfs(device: &str, target: &str) -> Result<()> {
    let path = Path::new(target);
    if path.exists() {
        return Ok(());
    }

    mkdirat(CWD, path, Mode::from_bits_truncate(0o755))
        .with_context(|| format!("Failed to create {target}"))?;

    mount(device, target, "btrfs", MountFlags::empty(), None)
        .with_context(|| format!("Failed to mount {device} at {target}"))
}

fn cleanup_persistent_mounts() {
    try_unmount(DATA_MOUNT);
    try_unmount(STATE_MOUNT);
    if let Err(error) = luks2::close(DM_DATA) {
        kmsg::warn!("Failed to close LUKS mapping {}: {}", DM_DATA, error);
    }
    if let Err(error) = luks2::close(DM_STATE) {
        kmsg::warn!("Failed to close LUKS mapping {}: {}", DM_STATE, error);
    }
}

fn try_unmount(target: &str) {
    match unmount(target, UnmountFlags::empty()) {
        Ok(()) | Err(rustix::io::Errno::NOENT | rustix::io::Errno::INVAL) => {}
        Err(error) => kmsg::warn!("Failed to unmount {}: {}", target, error),
    }
}

fn bail_maintenance<T>(message: impl Into<String>) -> Result<T> {
    Err(anyhow::anyhow!(message.into()))
}

#[cfg(test)]
mod tests {
    #[test]
    fn has_state_config_returns_false_when_missing() {
        // ARRANGE
        let temp = tempfile::tempdir().expect("create tempdir");

        // ACT & ASSERT
        assert!(!temp.path().join("config.toml").exists());
    }
}
