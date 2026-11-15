//! Confinement primitives for untrusted workloads.
//!
//! This crate owns the vocabulary a workload needs before it is allowed to
//! touch the host: which kernel facilities exist, which of them this build can
//! drive, which of them the machine refuses, and what a child process must do
//! between fork and exec. Phase 1 adds detection: the crate now probes the
//! machine for those facilities, but nothing here changes process state yet.
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
//! | `caps` | none yet | 2 |
//! | `limits` | none yet | 2 |
//! | `namespaces` | none yet | 3 |
//! | `mounts` | none yet | 3 |
//! | `rootfs` | none yet | 3 |
//! | `mock` | none yet | 3 |
//! | `cgroup` | none yet | 4 |
//! | `spawn` | none yet | 5 |
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

pub mod error;
pub mod probe;
pub mod resolve;
pub mod stage;
pub mod sys;
