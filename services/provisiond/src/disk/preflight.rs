//! Install preflight checks shared by the dry-run plan and the installer.

use std::path::Path;

use anyhow::{Context as _, Result, bail};
use rustix::fs::sync;

use super::attrs::{MOUNTS_PATH, Mount, read_mounts};
use super::mount::{get_disk_mounts, unmount_all};
use super::{
    Role, Targets, disk_is_non_empty, find_partition_device, has_state_partition, parent_disk,
    validate_block_device, validate_disk_size,
};

/// Outcome of one install preflight check.
pub(crate) struct Check {
    pub name: &'static str,
    pub passed: bool,
    pub detail: String,
}

impl Check {
    pub(crate) fn pass(name: &'static str, detail: String) -> Self {
        Self {
            name,
            passed: true,
            detail,
        }
    }

    pub(crate) fn fail(name: &'static str, detail: String) -> Self {
        Self {
            name,
            passed: false,
            detail,
        }
    }
}

/// Runs every preflight check against the resolved install targets.
pub(crate) fn checks(targets: &Targets, force: bool, allow_system_disk: bool) -> Vec<Check> {
    let mut checks = vec![not_installed(force)];
    checks.extend(disk_checks(&targets.system, force));
    if targets.is_split() {
        checks.extend(disk_checks(targets.data(), force));
    }
    checks.push(system_disk_in_use(
        &targets.system,
        running_system_disk().as_deref(),
        allow_system_disk,
    ));

    checks
}

/// Returns an error listing every refusing check, if any.
pub(crate) fn enforce(checks: &[Check]) -> Result<()> {
    let refused: Vec<&str> = checks
        .iter()
        .filter(|check| !check.passed)
        .map(|check| check.detail.as_str())
        .collect();

    if refused.is_empty() {
        return Ok(());
    }

    bail!("{}", refused.join("; "))
}

/// Unmounts every filesystem on the targets so partitioning can proceed.
pub(crate) fn unmount_targets(targets: &Targets) -> Result<()> {
    for device in [targets.system.as_str(), targets.data()] {
        let mounted = get_disk_mounts(device);
        if mounted.is_empty() {
            continue;
        }

        sync();
        unmount_all(&mounted).with_context(|| format!("Failed to unmount '{device}'"))?;
    }

    Ok(())
}

/// Returns the device path of the disk hosting the running system.
pub(crate) fn running_system_disk() -> Option<String> {
    if let Some(partition) = find_partition_device(Role::State)
        && let Some(disk) = parent_disk(&partition)
    {
        return Some(device_path(&disk));
    }

    let mounts = read_mounts(Path::new(MOUNTS_PATH));
    mount_disk(mounts.iter().find(|mount| mount.mount_point == "/"))
}

fn mount_disk(mount: Option<&Mount>) -> Option<String> {
    let disk = parent_disk(&mount?.device)?;
    let device = device_path(&disk);

    Path::new(&device).exists().then_some(device)
}

fn device_path(disk_name: &str) -> String {
    format!("/dev/{disk_name}")
}

fn disk_checks(device: &str, force: bool) -> Vec<Check> {
    let exists = device_exists(device);
    if !exists.passed {
        return vec![exists];
    }

    vec![
        exists,
        block_device(device),
        disk_size(device),
        not_mounted(device, force),
        no_muak_install(device, force),
        empty_disk(device, force),
    ]
}

fn not_installed(force: bool) -> Check {
    if force || !Path::new(config::system::CONFIG_PATH).exists() {
        return Check::pass("not_installed", "System is not installed".to_owned());
    }

    Check::fail(
        "not_installed",
        "Cannot install from an already-installed system. Boot from live ISO or use --force."
            .to_owned(),
    )
}

fn device_exists(device: &str) -> Check {
    if !Path::new(device).exists() {
        return Check::fail("device_exists", format!("Disk '{device}' does not exist"));
    }

    Check::pass("device_exists", format!("'{device}' exists"))
}

fn block_device(device: &str) -> Check {
    match validate_block_device(device) {
        Ok(()) => Check::pass("block_device", format!("'{device}' is a block device")),
        Err(e) => Check::fail("block_device", format!("{e:#}")),
    }
}

fn disk_size(device: &str) -> Check {
    match validate_disk_size(device) {
        Ok(()) => Check::pass("disk_size", format!("'{device}' meets the minimum size")),
        Err(e) => Check::fail("disk_size", format!("{e:#}")),
    }
}

fn not_mounted(device: &str, force: bool) -> Check {
    let mounted = get_disk_mounts(device);
    let first = mounted.first();

    match first {
        Some(mount) if !force => Check::fail(
            "not_mounted",
            format!(
                "Disk '{device}' is mounted at {}. Use --force to unmount automatically.",
                mount.mount_point
            ),
        ),
        Some(_) => Check::pass(
            "not_mounted",
            format!("'{device}' is mounted and will be unmounted"),
        ),
        None => Check::pass("not_mounted", format!("'{device}' is not mounted")),
    }
}

fn no_muak_install(device: &str, force: bool) -> Check {
    match has_state_partition(device) {
        Ok(true) if !force => Check::fail(
            "no_muak_install",
            format!(
                "Disk '{device}' already has a Muak installation (STATE partition found). \
                 Use --force to overwrite."
            ),
        ),
        Ok(true) => Check::pass(
            "no_muak_install",
            format!("'{device}' has a Muak installation that will be overwritten"),
        ),
        Ok(false) => Check::pass(
            "no_muak_install",
            format!("'{device}' has no Muak installation"),
        ),
        Err(e) => Check::fail("no_muak_install", format!("{e:#}")),
    }
}

fn empty_disk(device: &str, force: bool) -> Check {
    match disk_is_non_empty(device) {
        Ok(true) if !force => Check::fail(
            "empty_disk",
            format!(
                "Disk '{device}' is not empty and will be overwritten. Use --force to continue."
            ),
        ),
        Ok(true) => Check::pass("empty_disk", format!("'{device}' will be overwritten")),
        Ok(false) => Check::pass("empty_disk", format!("'{device}' is empty")),
        Err(e) => Check::fail("empty_disk", format!("{e:#}")),
    }
}

pub(crate) fn system_disk_in_use(device: &str, running: Option<&str>, allow: bool) -> Check {
    match running {
        Some(running) if running == device && allow => Check::pass(
            "system_disk_in_use",
            format!("'{device}' hosts the running system and --allow-system-disk was given"),
        ),
        Some(running) if running == device => Check::fail(
            "system_disk_in_use",
            format!(
                "Disk '{device}' hosts the running system. Use --allow-system-disk to destroy it."
            ),
        ),
        Some(running) => Check::pass(
            "system_disk_in_use",
            format!("'{device}' does not host the running system on '{running}'"),
        ),
        None => Check::pass(
            "system_disk_in_use",
            "The running system disk could not be determined".to_owned(),
        ),
    }
}

#[cfg(test)]
mod tests {
    use std::io::Write as _;

    use tempfile::NamedTempFile;

    use super::*;

    fn passing(name: &'static str) -> Check {
        Check::pass(name, "ok".to_owned())
    }

    #[test]
    fn device_checks_short_circuit_when_the_device_is_missing() {
        // ARRANGE
        let missing = "/dev/muak-missing";

        // ACT
        let checks = disk_checks(missing, false);

        // ASSERT
        assert_eq!(checks.len(), 1);
        assert_eq!(checks[0].name, "device_exists");
        assert!(!checks[0].passed);
    }

    #[test]
    fn device_checks_refuse_a_regular_file() {
        // ARRANGE
        let mut file = NamedTempFile::new().expect("temp file");
        file.write_all(b"not a disk").expect("write");

        // ACT
        let checks = disk_checks(file.path().to_str().expect("utf8"), false);

        // ASSERT
        let block_device = checks
            .iter()
            .find(|check| check.name == "block_device")
            .expect("block device check");
        assert!(!block_device.passed, "{}", block_device.detail);
        assert!(block_device.detail.contains("not a block device"));
    }

    #[test]
    fn system_disk_in_use_refuses_the_running_disk() {
        // ARRANGE
        let running = Some("/dev/sda");

        // ACT
        let check = system_disk_in_use("/dev/sda", running, false);

        // ASSERT
        assert!(!check.passed);
        assert!(check.detail.contains("--allow-system-disk"));
    }

    #[test]
    fn system_disk_in_use_allows_the_running_disk_when_flagged() {
        // ARRANGE
        let running = Some("/dev/sda");

        // ACT
        let check = system_disk_in_use("/dev/sda", running, true);

        // ASSERT
        assert!(check.passed, "{}", check.detail);
        assert!(check.detail.contains("--allow-system-disk"));
    }

    #[test]
    fn system_disk_in_use_ignores_other_disks() {
        // ARRANGE
        let running = Some("/dev/nvme0n1");

        // ACT
        let check = system_disk_in_use("/dev/sda", running, false);

        // ASSERT
        assert!(check.passed, "{}", check.detail);
    }

    #[test]
    fn system_disk_in_use_passes_when_the_running_disk_is_unknown() {
        // ARRANGE
        let running: Option<&str> = None;

        // ACT
        let check = system_disk_in_use("/dev/sda", running, false);

        // ASSERT
        assert!(check.passed, "{}", check.detail);
        assert!(check.detail.contains("could not be determined"));
    }

    #[test]
    fn enforce_accepts_passing_checks() {
        // ARRANGE
        let checks = vec![passing("device_exists"), passing("disk_size")];

        // ACT
        let result = enforce(&checks);

        // ASSERT
        result.expect("passing checks must be accepted");
    }

    #[test]
    fn enforce_reports_every_refusal() {
        // ARRANGE
        let checks = vec![
            passing("device_exists"),
            Check::fail("disk_size", "too small".to_owned()),
            Check::fail("empty_disk", "not empty".to_owned()),
        ];

        // ACT
        let error = enforce(&checks).expect_err("refusals must fail");

        // ASSERT
        let message = error.to_string();
        assert!(message.contains("too small"), "{message}");
        assert!(message.contains("not empty"), "{message}");
    }
}
