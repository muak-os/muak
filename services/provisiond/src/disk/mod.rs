//! Disk management utilities for partitioning and formatting.

pub(crate) mod apply;
mod attrs;
mod blkpg;
pub(crate) mod checks;
mod constants;
mod foreign;
pub(crate) mod format;
mod gpt;
pub(crate) mod guard;
pub(crate) mod manage;
pub(crate) mod mount;
mod pattern;
mod select;
pub(crate) mod size;
pub(crate) mod sysfs;
pub(crate) mod target;
pub(crate) mod types;
pub(crate) mod validate;
pub(crate) mod wipe;

use std::path::Path;

use ::disk::plan::Document;
use anyhow::{Context as _, Result};

/// Path of the disk document on a booted, installed system.
pub(crate) const PLAN_BOOT_PATH: &str = "/run/boot/disk.toml";

/// Loads the disk document carried by the booted image.
pub(crate) fn load_document() -> Result<Document> {
    Document::read(Path::new(PLAN_BOOT_PATH))
        .with_context(|| format!("failed to read the boot disk document {PLAN_BOOT_PATH}"))
}
