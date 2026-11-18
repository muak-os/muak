//! Confinement primitives for untrusted workloads.
//!
//! This crate owns the vocabulary a workload needs before it is allowed to
//! touch the host: which kernel facilities exist, which of them this build can
//! drive, which of them the machine refuses, and what a child process must do
//! between fork and exec. Phase 4 adds namespaces, mounts and the root
//! filesystem switch that the child performs before the capability drop.
//! Phase 5 adds the cgroup v2 tree the child is born in.
//!
//! Control group v1 is deliberately unsupported. The kernel boots with
//! `cgroup_no_v1=all`, so a v1 hierarchy can never appear, and every facility
//! this crate relies on is a v2 facility.
//!
//! # Features
//!
//! A row that gates nothing yet is deliberate staging: the feature already
//! exists so callers can select it, and the phase column says when the code
//! behind it lands.
//!
//! | Feature | Module or step it gats | Phase |
//! | --- | --- | --- |
//! | `linux` | `sys::linux` | 0 |
//! | `probe` | `probe::detect`, `sys::linux::probe` | 1 |
//! | `caps` | the capability calls in `sys::linux::credentials` (also needs `spawn`) | 2 |
//! | `limits` | `sys::linux::limits` (also needs `spawn`) | 2 |
//! | `namespaces` | `namespace::apply`, `sys::linux::namespace` | 4 |
//! | `mounts` | `mount::apply`, `sys::linux::mount` | 4 |
//! | `rootfs` | `rootfs::apply`, `sys::linux::rootfs` | 4 |
//! | `mock` | none yet | 3 |
//! | `cgroupv2` | `cgroup::v2` | 5 |
//! | `spawn` | `resolve::Resolved`, `Policy::resolve`, `child`, `spawn`, `sys::linux::spawn` | 2 |
//! | `seccomp` | none yet | 6 |
//! | `overlay` | none yet | 7 |
//!
//! # Child path discipline
//!
//! Code reachable between fork and exec is allocation free, panic free and lock
//! free, it only borrows what it was given, and it reports failures with
//! `_exit` instead of `std::process::exit`. A forked child of a multithreaded
//! parent may inherit a held allocator or panic lock whose owner does not exist
//! in the child, so allocating or unwinding there deadlocks instead of
//! reporting. `std::process::exit` runs destructors and flushes stdio buffers
//! that belong to the parent, which duplicates or loses output the parent still
//! owns.

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
