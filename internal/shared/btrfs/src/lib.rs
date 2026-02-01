//! Btrfs filesystem creation and manipulation.
//!
//! This library provides functionality for:
//! - Creating Btrfs filesystems from scratch
//! - Managing subvolumes (create, delete, list)
//! - Managing quotas (enable, set limits, get usage)

mod error;
pub mod format;
pub mod ioctl;
pub mod quota;
pub mod subvolume;

pub use error::{BtrfsError, Result};
pub use format::{format_btrfs, get_device_size};
pub use quota::{DiskUsage, enable_quota, get_usage, set_quota};
pub use subvolume::{create_subvolume, delete_subvolume, list_subvolumes};
