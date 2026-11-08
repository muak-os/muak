//! The block devices and partitions the kernel reports.

#![warn(missing_docs)]

#[cfg(not(feature = "linux"))]
compile_error!("blockdev requires a platform feature; enable `linux`");

#[cfg(all(feature = "linux", not(target_os = "linux")))]
compile_error!("the `linux` blockdev feature requires a Linux target");

pub mod error;
pub mod io;
pub mod types;

#[cfg(all(feature = "linux", target_os = "linux"))]
pub mod device;
#[cfg(all(feature = "linux", target_os = "linux"))]
pub mod discover;
#[cfg(all(feature = "linux", target_os = "linux"))]
pub mod holders;
#[cfg(all(feature = "linux", target_os = "linux"))]
pub mod inventory;
#[cfg(all(feature = "linux", target_os = "linux"))]
pub mod mounts;
#[cfg(all(feature = "linux", target_os = "linux"))]
pub mod roots;

#[cfg(all(feature = "linux", target_os = "linux"))]
pub(crate) mod attrs;
#[cfg(all(feature = "linux", target_os = "linux"))]
pub(crate) mod partitions;
#[cfg(all(feature = "linux", target_os = "linux"))]
pub(crate) mod probe;
