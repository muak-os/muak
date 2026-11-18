//! Confinement primitives for untrusted workloads.
//!
//! Control group v1 is deliberately unsupported: the kernel boots with `cgroup_no_v1=all`.
//!
//! Code between fork and exec is allocation, panic and lock free: a forked child of a multithreaded parent can inherit a held allocator or panic lock and deadlock.

#![warn(missing_docs)]

pub mod cap;
pub mod cgroup;
pub mod credentials;
pub mod error;
pub mod limits;
pub mod mount;
pub mod namespace;
pub mod policy;
pub mod probe;
pub mod resolve;
pub mod rootfs;
#[cfg(feature = "spawn")]
pub mod spawn;
pub mod stage;
pub mod sys;

#[cfg(feature = "spawn")]
pub(crate) mod child;
