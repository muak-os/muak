//! Boundary between the sandbox crate and the kernel.
//!
//! This is the only module of the crate that issues raw syscalls: every other module decides what a workload gets and how a failure is reported.

#[cfg(feature = "linux")]
pub mod linux;
