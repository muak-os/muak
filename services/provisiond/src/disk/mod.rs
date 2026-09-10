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
pub(crate) use ::disk::doc::{Doc, partition_name};
pub(crate) use ::disk::role::Role;
pub(crate) use anyhow::{Context as _, Result};
pub(crate) use apply::apply_plan;
pub(crate) use format::{format_btrfs_partition, format_efi_partition};
pub(crate) use manage::{delete_partitions, find_partition_number, has_state_partition};
pub(crate) use mount::{mount_efi_partition, try_unmount, unmount_partition};
pub(crate) use sysfs::{list_disks, validate_block_device, validate_disk_size};
pub(crate) use validate::install_target;

/// File name of the disk document on the STATE partition.
pub(crate) const DOC_FILE: &str = "disk.toml";

/// Path of the disk document on a booted, installed system.
pub(crate) const DOC_STATE_PATH: &str = "/run/state/disk.toml";

/// Writes the disk document into a mounted STATE partition directory.
pub(crate) fn write_doc(mount_point: &Path, doc: &Doc) -> Result<()> {
    doc.write(&mount_point.join(DOC_FILE))
        .context("Failed to write disk document")
}

/// Loads the disk document from the installed STATE partition.
pub(crate) fn load_installed_doc() -> Result<Option<Doc>> {
    if !Path::new(DOC_STATE_PATH).exists() {
        return Ok(None);
    }

    Doc::read(Path::new(DOC_STATE_PATH))
        .context("Failed to read disk document")
        .map(Some)
}
