//! Disk management utilities for partitioning and formatting.

mod apply;
mod blkpg;
mod constants;
mod format;
mod gpt;
mod manage;
mod mount;
mod sysfs;
mod types;
mod validate;
mod wipe;

use std::path::Path;

pub(crate) use ::disk::discover::find_partition_device;
pub(crate) use ::disk::plan::Document;
pub(crate) use ::disk::role::Role;
pub(crate) use anyhow::{Context as _, Result};
pub(crate) use apply::apply_plan;
pub(crate) use format::{format_btrfs_partition, format_efi_partition};
pub(crate) use manage::{delete_partitions, find_partition_number, has_state_partition};
pub(crate) use mount::{mount_efi_partition, try_unmount, unmount_partition};
pub(crate) use sysfs::{list_disks, validate_block_device, validate_disk_size};
pub(crate) use validate::install_target;

/// Path of the disk document on a booted, installed system.
pub(crate) const PLAN_BOOT_PATH: &str = "/run/boot/disk.toml";

/// Loads the disk document carried by the booted image.
pub(crate) fn load_document() -> Result<Document> {
    Document::read(Path::new(PLAN_BOOT_PATH))
        .with_context(|| format!("failed to read the boot disk document {PLAN_BOOT_PATH}"))
}
