//! Boundary between the sandbox crate and the kernel.
//!
//! This is the only module of the crate that issues raw syscalls. Every other
//! module is policy and orchestration: it decides what a workload gets and how
//! a failure is reported, while this one performs the calls. The recording
//! backend that lets tests replay a child path lands in Phase 3.

#[cfg(feature = "linux")]
pub mod linux;
