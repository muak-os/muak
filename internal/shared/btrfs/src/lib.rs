//! Btrfs filesystem creation and manipulation.
//!
//! This library provides functionality for:
//! - Creating Btrfs filesystems from scratch
//! - Managing subvolumes (create, delete, list)
//! - Managing quotas (enable, set limits, get usage)
//! - Scrubbing filesystems for integrity verification

mod error;
pub mod format;
pub mod ioctl;
pub mod quota;
pub mod scrub;
pub mod subvolume;

pub use error::{BtrfsError, Result};
pub use format::{format_btrfs, get_device_size};
pub use ioctl::BtrfsScrubProgress;
pub use quota::{DiskUsage, enable_quota, get_usage, set_quota};
pub use scrub::{get_fs_info, scrub};
pub use subvolume::{create_subvolume, delete_subvolume, list_subvolumes};
