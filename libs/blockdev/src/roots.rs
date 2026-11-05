//! Filesystem roots the inventory is read from.

use std::path::PathBuf;

/// Filesystem roots consulted when reading the block device inventory.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Roots {
    /// One directory per disk, normally `/sys/block`.
    pub block: PathBuf,
    /// Flat class tree with disks and partitions side by side, normally `/sys/class/block`.
    pub class_block: PathBuf,
    /// Device node directory, normally `/dev`.
    pub dev: PathBuf,
    /// Kernel mount table, normally `/proc/mounts`.
    pub mounts: PathBuf,
    /// Kernel swap table, normally `/proc/swaps`.
    pub swaps: PathBuf,
}

impl Roots {
    /// Returns the roots of the running system.
    #[must_use]
    pub fn system() -> Self {
        Self {
            block: PathBuf::from("/sys/block"),
            class_block: PathBuf::from("/sys/class/block"),
            dev: PathBuf::from("/dev"),
            mounts: PathBuf::from("/proc/mounts"),
            swaps: PathBuf::from("/proc/swaps"),
        }
    }
}
