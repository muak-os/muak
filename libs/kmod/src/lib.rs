//! Kernel module management library.
//!
//! Provides utilities for loading kernel modules, resolving dependencies,
//! and managing module aliases.

#![warn(missing_docs)]

pub mod aliases;
pub mod deps;
pub mod kernel;
pub mod sysfs;
