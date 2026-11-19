//! Confinement primitives for untrusted workloads.

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
