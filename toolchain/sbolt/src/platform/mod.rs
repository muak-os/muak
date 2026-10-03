//! Platform firmware variable backends.

#[cfg(all(feature = "linux", target_os = "linux"))]
pub(crate) mod linux;
