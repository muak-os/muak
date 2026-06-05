//! Btrfs filesystem creation and manipulation.
//!
//! This library provides functionality for:
//! - Creating Btrfs filesystems from scratch
//! - Managing subvolumes (create, delete, list)
//! - Managing quotas (enable, set limits, get usage)
//! - Scrubbing filesystems for integrity verification

#![warn(missing_docs)]

pub mod error;
pub mod format;
pub mod ioctl;
pub mod quota;
pub mod scrub;
pub mod subvolume;
