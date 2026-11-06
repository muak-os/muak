use std::fs::OpenOptions;

use ::disk::role::Role;
use anyhow::Result;
use partdev::{blkpg, clear, manage, table};

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

    if needs_release(target) {
        release(target)?;
    }

    kmsg::info!(
        "Removing all existing partitions from kernel for {}",
        target.path
    );
    let removed = blkpg::delete_all(&target.path)?;
    super::log_removed(&target.path, &removed);

    if options.drop_partition && !target.partitions.is_empty() {
        drop_entries(target)?;
    }

    write(&target.path, options.method)?;

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

fn needs_release(target: &Target) -> bool {
    !get_disk_mounts(&target.path).is_empty() || target.is_system || target.is_data
}

fn release(target: &Target) -> Result<()> {
    let mounts = get_disk_mounts(&target.path);
    unmount_all(&mounts)?;

    if target.is_system {
        close_mapping(Role::State);
        close_mapping(Role::Ephemeral);
    }
    if target.is_data {
        close_mapping(Role::Data);
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
        kmsg::info!(
            "No GPT on {}, kernel partitions dropped, wipe will remove any table",
            target.path
        );
        return Ok(());
    }
    drop(file);

    kmsg::info!(
        "Deleting partitions {:?} from GPT on {}",
        target.partitions,
        target.path
    );
    let removed = manage::delete_partitions(&target.path, &target.partitions)?;
    super::log_removed(&target.path, &removed);

    Ok(())
}

fn write(disk: &str, method: Method) -> Result<()> {
    match method {
        Method::Fast => clear::fast(disk)?,
        Method::Zeroes => clear::zero(disk)?,
    }

    Ok(())
}
