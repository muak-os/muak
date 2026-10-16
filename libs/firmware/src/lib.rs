//! Firmware abstraction library.

#![warn(missing_docs)]

#[cfg(not(feature = "linux"))]
compile_error!("firmware requires a platform feature; enable `linux`");

#[cfg(all(feature = "linux", not(target_os = "linux")))]
compile_error!("the `linux` firmware feature requires a Linux target");

pub mod error;
pub mod esrt;
pub mod identity;
pub mod inventory;
pub mod nvram;
pub mod platform;
pub mod secureboot;

#[cfg(all(feature = "linux", target_os = "linux"))]
pub mod linux;

/// Platform implementation chosen for this build.
#[cfg(all(feature = "linux", target_os = "linux"))]
pub type System = linux::Linux;

/// Returns a handle to the active platform implementation.
#[must_use]
pub fn system() -> System {
    System::new()
}
