//! Raw packet sockets.

#![warn(missing_docs)]

extern crate alloc;

pub mod socket;
#[cfg(feature = "v4")]
pub mod v4;

#[cfg(not(feature = "v4"))]
compile_error!("enable the v4 feature");
