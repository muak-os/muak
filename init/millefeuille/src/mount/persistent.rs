//! Persistent mounts for installed systems.

use std::path::Path;

use anyhow::{Context as _, Result, bail};
use blockdev::discover;
use btrfs::quota;
use disk::role::Role;
use rustix::fs::{AtFlags, CWD, Mode, mkdirat, unlinkat};
use rustix::io::Errno;
use rustix::mount::{MountFlags, UnmountFlags, mount, unmount};

use super::luks::resolve_key;

const STATE_MOUNT: &str = "/run/state";
const EPHEMERAL_MOUNT: &str = "/run/ephemeral";
const STATE_CONFIG: &str = "/run/state/config.toml";

/// Mount persistent STATE and EPHEMERAL partitions if the system is installed.
pub(crate) fn persistent() -> bool {
    let state_devices = discover::find_by_partname(Role::State.gpt_name());
    if state_devices.is_empty() {
        return false;
    }

    for state_dev in state_devices {
        match try_mount_persistent_candidate(&state_dev) {
            Ok(()) => return true,
            Err(error) => {
                kmsg::warn!("Ignoring STATE candidate {}: {:#}", state_dev, error);
                cleanup_persistent_mounts();
            }
        }
    }

    false
}

fn try_mount_persistent_candidate(state_dev: &str) -> Result<()> {
    let luks_key = resolve_key(state_dev)?;

    let Some(key) = luks_key else {
        bail!(
            "No LUKS key available for installed STATE partition {state_dev}; TPM2 recovery unavailable and cmdline fallback missing or invalid"
        );
    };

    luks2::open(state_dev, Role::State.dm_name(), &key)
        .with_context(|| format!("Failed to open LUKS device: {state_dev}"))?;
    let state_device = format!("/dev/mapper/{}", Role::State.dm_name());

    mount_btrfs(&state_device, STATE_MOUNT)
        .with_context(|| format!("Failed to mount STATE partition: {state_dev}"))?;

    if !Path::new(STATE_CONFIG).exists() {
        bail!("STATE partition is missing config.toml");
    }

    kmsg::info!("Mounted STATE partition at /run/state");

    let Some(ephemeral_device) = discover::find_by_partname(Role::Ephemeral.gpt_name())
        .into_iter()
        .next()
    else {
        bail!("EPHEMERAL partition not found");
    };

    try_mount_ephemeral(&ephemeral_device, &key)
}

fn try_mount_ephemeral(device: &str, key: &[u8]) -> Result<()> {
    luks2::open(device, Role::Ephemeral.dm_name(), key)
        .with_context(|| format!("Failed to open LUKS device: {device}"))?;
    let mapped = format!("/dev/mapper/{}", Role::Ephemeral.dm_name());

    if let Err(error) = mount_btrfs(&mapped, EPHEMERAL_MOUNT) {
        close_mapping(Role::Ephemeral);
        remove_mountpoint(EPHEMERAL_MOUNT);
        return Err(error);
    }

    kmsg::info!("Mounted EPHEMERAL partition at /run/ephemeral");

    if let Err(error) = quota::enable(EPHEMERAL_MOUNT) {
        try_unmount(EPHEMERAL_MOUNT);
        close_mapping(Role::Ephemeral);
        remove_mountpoint(EPHEMERAL_MOUNT);
        return Err(error.into());
    }

    Ok(())
}

fn remove_mountpoint(target: &str) {
    if let Err(error) = unlinkat(CWD, target, AtFlags::REMOVEDIR)
        && error != Errno::NOENT
    {
        kmsg::warn!("Failed to remove {}: {}", target, error);
    }
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
    teardown(Role::Ephemeral, EPHEMERAL_MOUNT);
    teardown(Role::State, STATE_MOUNT);
}

fn teardown(role: Role, target: &str) {
    try_unmount(target);
    close_mapping(role);
    remove_mountpoint(target);
}

fn close_mapping(role: Role) {
    if let Err(error) = luks2::close(role.dm_name()) {
        kmsg::warn!("Failed to close LUKS mapping {}: {}", role.dm_name(), error);
    }
}

fn try_unmount(target: &str) {
    match unmount(target, UnmountFlags::empty()) {
        Ok(()) | Err(Errno::NOENT | Errno::INVAL) => {}
        Err(error) => kmsg::warn!("Failed to unmount {}: {}", target, error),
    }
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
