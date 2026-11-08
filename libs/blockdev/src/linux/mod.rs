//! Linux kernel view over sysfs, procfs and block ioctls.

mod attrs;
mod device;
mod discover;
mod holders;
mod inventory;
mod mounts;
mod partitions;
mod probe;

use std::os::fd::AsFd;
use std::path::PathBuf;

use crate::error::Result;
use crate::kernel::Kernel;
use crate::mounts::{Mount, Swap};
use crate::types::Disk;

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

/// Linux implementation of the kernel view.
#[derive(Debug, Clone)]
pub struct Linux {
    roots: Roots,
}

impl Linux {
    /// Creates an implementation rooted at the system sysfs and procfs.
    #[must_use]
    pub fn new() -> Self {
        Self::rooted(&Roots::system())
    }

    /// Creates an implementation rooted at `roots`, for tests and alternate mounts.
    #[must_use]
    pub fn rooted(roots: &Roots) -> Self {
        Self {
            roots: roots.clone(),
        }
    }
}

impl Default for Linux {
    fn default() -> Self {
        Self::new()
    }
}

impl Kernel for Linux {
    fn disks(&self) -> Result<Vec<Disk>> {
        inventory::list_in(&self.roots)
    }

    fn find_by_partname(&self, partname: &str) -> Vec<String> {
        discover::find_by_partname_in(&self.roots, partname)
    }

    fn holders(&self, device: &str) -> Result<Vec<String>> {
        holders::try_names(&self.roots, device)
    }

    fn mounts(&self) -> Vec<Mount> {
        crate::mounts::read(&self.roots.mounts)
    }

    fn parent_disk(&self, partition_device: &str) -> Option<String> {
        discover::parent_disk_in(&self.roots, partition_device)
    }

    fn size<Fd: AsFd>(&self, fd: &Fd) -> Result<u64> {
        device::size(fd)
    }

    fn swaps(&self) -> Result<Vec<Swap>> {
        mounts::try_read_swaps(&self.roots.swaps)
    }
}
