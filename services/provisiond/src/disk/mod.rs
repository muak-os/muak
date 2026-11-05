//! Disk management utilities for partitioning and formatting.

pub(crate) mod apply;
pub(crate) mod checks;
mod constants;
mod foreign;
pub(crate) mod format;
mod gpt;
pub(crate) mod guard;
pub(crate) mod mount;
mod pattern;
pub(crate) mod roles;
mod select;
pub(crate) mod target;
pub(crate) mod validate;
pub(crate) mod wipe;

use std::path::Path;

use ::disk::plan::Document;
use anyhow::{Context as _, Result};
use blockdev::inventory;
use blockdev::types::Disk;
use partdev::blkpg;

/// Path of the disk document on a booted, installed system.
pub(crate) const PLAN_BOOT_PATH: &str = "/run/boot/disk.toml";

/// Lists the disks reported by the kernel without blocking the async runtime.
pub(crate) async fn list_disks() -> Result<Vec<Disk>> {
    tokio::task::spawn_blocking(inventory::list)
        .await
        .context("disk inventory task")?
        .context("failed to read the disk inventory")
}

/// Loads the disk document carried by the booted image.
pub(crate) fn load_document() -> Result<Document> {
    Document::read(Path::new(PLAN_BOOT_PATH))
        .with_context(|| format!("failed to read the boot disk document {PLAN_BOOT_PATH}"))
}

/// Logs the kernel outcome of each partition removed from `disk`.
pub(crate) fn log_removed(disk: &str, removed: &[(u32, blkpg::Removed)]) {
    for &(number, outcome) in removed {
        match outcome {
            blkpg::Removed::Present => {
                kmsg::info!(
                    "BLKPG: Successfully removed partition {} from {}",
                    number,
                    disk
                );
            }
            blkpg::Removed::Absent => {
                kmsg::info!(
                    "BLKPG: Partition {} not present in kernel on {} (OK)",
                    number,
                    disk
                );
            }
        }
    }
}
