//! Read, write it, and keep the kernel's view in sync of the partition table.

#![warn(missing_docs)]

#[cfg(not(feature = "linux"))]
compile_error!("partdev requires a platform feature; enable `linux`");

#[cfg(all(feature = "linux", not(target_os = "linux")))]
compile_error!("the `linux` partdev feature requires a Linux target");

pub mod clear;
pub mod error;
pub mod kernel;
pub mod manage;
pub mod table;

#[cfg(all(feature = "linux", target_os = "linux"))]
pub mod linux;

/// Returns a handle to the kernel implementation chosen for this build.
#[cfg(all(feature = "linux", target_os = "linux"))]
#[must_use]
pub(crate) fn handle() -> impl kernel::Kernel {
    linux::Linux::new()
}
