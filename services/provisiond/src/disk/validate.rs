use std::io::{Seek as _, SeekFrom};
use std::path::Path;

use ::disk::plan::{Plan, Size};
use anyhow::{Context as _, Result, bail};
use rustix::fs::sync;

use super::constants::MB;

/// Validates that a disk is large enough for the plan.
///
/// # Errors
///
/// Returns an error when the disk is smaller than the plan requires.
pub(crate) fn fits_plan(disk: &str, plan: &Plan) -> Result<()> {
    let available = disk_size(disk)?;
    let required = required_bytes(plan);

    if available < required {
        bail!(
            "Disk '{disk}' is too small for the layout: {} MiB required, {} MiB available",
            required.checked_div(MB).unwrap_or(0),
            available.checked_div(MB).unwrap_or(0)
        );
    }

    Ok(())
}

fn required_bytes(plan: &Plan) -> u64 {
    let fill_count = u64::try_from(
        plan.partitions
            .iter()
            .filter(|spec| spec.size == Size::Fill)
            .count(),
    )
    .unwrap_or(u64::MAX);

    let fixed: u64 = plan
        .partitions
        .iter()
        .map(|spec| match spec.size {
            Size::Fixed(bytes) => bytes,
            Size::Fill => 0,
        })
        .sum();

    fixed
        .saturating_add(fill_count.saturating_mul(MB))
        .saturating_add(MB)
}

fn disk_size(disk: &str) -> Result<u64> {
    let mut file = std::fs::File::open(disk)
        .with_context(|| format!("Failed to open '{disk}' for size probe"))?;

    Ok(file.seek(SeekFrom::End(0))?)
}

/// Validates that the system and data disks are suitable install targets.
pub fn install_target(system_disk: &str, data_disk: &str, force: bool) -> Result<()> {
    if !force && Path::new(config::CONFIG_PATH).exists() {
        bail!(
            "Cannot install from an already-installed system. Boot from live ISO or use --force."
        );
    }

    disk(system_disk, force)
        .with_context(|| format!("System disk '{system_disk}' failed validation"))?;

    if data_disk != system_disk {
        disk(data_disk, force)
            .with_context(|| format!("Data disk '{data_disk}' failed validation"))?;
    }

    Ok(())
}

/// Validates a disk as a suitable install target.
fn disk(disk_path: &str, force: bool) -> Result<()> {
    if !Path::new(disk_path).exists() {
        bail!("Disk '{disk_path}' does not exist");
    }

    super::validate_block_device(disk_path)?;
    super::validate_disk_size(disk_path)?;

    let mounted = super::mount::get_disk_mounts(disk_path);
    if !mounted.is_empty()
        && !force
        && let Some(first) = mounted.first()
    {
        bail!(
            "Cannot install: {} is mounted at {}. Use --force to unmount automatically.",
            first.device,
            first.mount_point
        );
    }

    sync();
    super::mount::unmount_all(&mounted)?;

    let has_state_partition = super::has_state_partition(disk_path)?;
    if has_state_partition && !force {
        bail!(
            "Disk '{disk_path}' already has a Muak installation (STATE partition found). \
             Use --force to overwrite."
        );
    }

    if super::manage::disk_is_non_empty(disk_path)? && !force {
        bail!("Disk '{disk_path}' is not empty and will be overwritten. Use --force to continue.");
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use ::disk::layout::Layout;

    use super::*;

    #[test]
    fn required_bytes_covers_fixed_partitions_and_fill_floor() {
        // ARRANGE
        let plan = Layout::Uefi.plan();

        // ACT
        let required = required_bytes(&plan);

        // ASSERT
        let fixed: u64 = plan
            .partitions
            .iter()
            .map(|spec| match spec.size {
                Size::Fixed(bytes) => bytes,
                Size::Fill => 0,
            })
            .sum();
        assert_eq!(required, fixed + MB * 2, "fill floor plus GPT margin");
    }

    #[test]
    fn required_bytes_is_zero_for_an_empty_plan() {
        // ARRANGE
        let plan = Plan::wiped(Vec::new());

        // ACT
        let required = required_bytes(&plan);

        // ASSERT
        assert_eq!(required, MB, "only the GPT margin remains");
    }
}
