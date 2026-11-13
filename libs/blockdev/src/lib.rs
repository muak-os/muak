//! The block devices and partitions the kernel reports.

#![warn(missing_docs)]

#[cfg(all(feature = "linux", not(target_os = "linux")))]
compile_error!("the `linux` blockdev feature requires a Linux target");

pub mod error;
pub mod io;
pub mod kernel;
pub mod mounts;
pub mod types;

#[cfg(all(feature = "linux", target_os = "linux"))]
pub mod device;
#[cfg(all(feature = "linux", target_os = "linux"))]
pub mod discover;
#[cfg(all(feature = "linux", target_os = "linux"))]
pub mod inventory;

#[cfg(all(feature = "linux", target_os = "linux"))]
pub mod linux;

/// Returns a handle to the kernel implementation chosen for this build.
#[cfg(all(feature = "linux", target_os = "linux"))]
#[must_use = "the handle is the only way to reach the kernel implementation"]
pub fn handle() -> impl kernel::Kernel {
    linux::Linux::new()
}
