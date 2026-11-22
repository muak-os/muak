//! Address provisioning methods hosted by the controller.

#[cfg(feature = "ipv4")]
pub mod v4;
#[cfg(feature = "ipv6")]
pub mod v6;
