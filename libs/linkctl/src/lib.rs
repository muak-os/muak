//! Family-neutral per-interface address configuration controller.

#![warn(missing_docs)]

extern crate alloc;

pub mod interface;
pub mod provision;
pub mod transport;
pub mod view;

#[cfg(not(any(feature = "ipv4", feature = "ipv6")))]
compile_error!("enable at least one of the v4 or v6 features");
