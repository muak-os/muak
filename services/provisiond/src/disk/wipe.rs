use std::fs::OpenOptions;

use ::disk::role::Role;
use anyhow::Result;
use blockdev::kernel::Kernel;
use partdev::{clear, kernel, manage, table};

use super::guard::Target;
use super::mount::{get_disk_mounts, unmount_all};

/// How much of the disk the wipe overwrites.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Method {
    /// Zero the head and tail of the disk, destroying the partition table.
    Fast,
    /// Zero the whole disk.
    Zeroes,
}

/// Options controlling a disk wipe.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Options {
    pub method: Method,
    pub drop_partition: bool,
    pub dry_run: bool,
}

/// Validates the target, releases it and wipes the requested regions.
pub(crate) fn run(target: &Target, options: Options) -> Result<String> {
    let summary = summary(target, options);
    if options.dry_run {
        return Ok(summary);
    }

    let kernel = blockdev::handle();
    if needs_release(&kernel, target) {
        release(&kernel, target)?;
    }

    if options.drop_partition && !target.partitions.is_empty() {
        drop_entries(target)?;
    }

    kmsg::info!("Preflight table re-read of {} before wiping", target.path);
    kernel::rescan(&target.path)?;
    write(&target.path, options.method)?;

    kmsg::info!(
        "Re-reading partition table of {} after the wipe",
        target.path
    );
    kernel::rescan(&target.path)?;

    Ok(summary)
}

fn summary(target: &Target, options: Options) -> String {
    let method = match options.method {
        Method::Fast => "fast, head and tail",
        Method::Zeroes => "zeroes, whole disk",
    };
    let drop = if options.drop_partition {
        " and drop its partition entries"
    } else {
        ""
    };

    if options.dry_run {
        format!("Would wipe {} ({method}){drop}", target.path)
    } else {
        format!("Wiped {} ({method}){drop}", target.path)
    }
}

fn needs_release<K: Kernel>(kernel: &K, target: &Target) -> bool {
    !get_disk_mounts(kernel, &target.path).is_empty() || target.is_system
}

fn release<K: Kernel>(kernel: &K, target: &Target) -> Result<()> {
    let mounts = get_disk_mounts(kernel, &target.path);
    unmount_all(&mounts)?;

    if target.is_system {
        close_mapping(Role::State);
        close_mapping(Role::Ephemeral);
    }

    Ok(())
}

fn close_mapping(role: Role) {
    if let Err(e) = luks2::close(role.dm_name()) {
        kmsg::warn!("Failed to close LUKS {} mapping: {}", role.dm_name(), e);
    }
}

fn drop_entries(target: &Target) -> Result<()> {
    let file = OpenOptions::new().read(true).open(&target.path)?;
    if table::read(&file).is_err() {
        kmsg::info!("No GPT on {}, the wipe will remove any table", target.path);
        return Ok(());
    }
    drop(file);

    kmsg::info!(
        "Deleting partitions {:?} from GPT on {}",
        target.partitions,
        target.path
    );
    manage::delete_partitions(&target.path, &target.partitions)?;

    Ok(())
}

fn write(disk: &str, method: Method) -> Result<()> {
    match method {
        Method::Fast => clear::fast(disk)?,
        Method::Zeroes => clear::zero(disk)?,
    }

    Ok(())
}
