//! Disk management utilities for partitioning and formatting.

mod apply;
mod attrs;
mod blkpg;
pub(crate) mod checks;
mod constants;
mod foreign;
mod format;
mod gpt;
mod guard;
mod manage;
mod mount;
mod pattern;
mod select;
mod sysfs;
mod target;
mod types;
mod validate;
mod wipe;

use std::path::Path;

pub(crate) use ::disk::discover::{find_partition_device, parent_disk, parent_disk_in};
pub(crate) use ::disk::mount::{
    efi as mount_efi_partition, ephemeral, try_unmount, unmount as unmount_partition,
};
pub(crate) use ::disk::plan::Document;
pub(crate) use ::disk::role::Role;
pub(crate) use anyhow::{Context as _, Result};
pub(crate) use apply::apply_plan;
pub(crate) use format::{format_btrfs_partition, format_efi_partition};
pub(crate) use guard::validate as validate_wipe;
pub(crate) use manage::{
    delete_partitions, disk_is_non_empty, find_partition_number, has_state_partition,
};
pub(crate) use mount::unmount_targets;
pub(crate) use sysfs::{list_disks, validate_block_device, validate_disk_size};
pub(crate) use target::{Targets, resolve, resolve_system, resolve_with};
pub(crate) use types::{DiskInfo, PartitionInfo};
pub(crate) use validate::fits_plan;
pub(crate) use wipe::{Method as WipeMethod, Options as WipeOptions, run as run_wipe};

/// Path of the disk document on a booted, installed system.
pub(crate) const PLAN_BOOT_PATH: &str = "/run/boot/disk.toml";

/// Loads the disk document carried by the booted image.
pub(crate) fn load_document() -> Result<Document> {
    Document::read(Path::new(PLAN_BOOT_PATH))
        .with_context(|| format!("failed to read the boot disk document {PLAN_BOOT_PATH}"))
}
