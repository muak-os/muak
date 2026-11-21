//! Raw packet sockets.

#![warn(missing_docs)]

extern crate alloc;

#[cfg(feature = "v6")]
pub mod icmpv6;
pub mod socket;
#[cfg(feature = "v4")]
pub mod v4;

#[cfg(not(any(feature = "v4", feature = "v6")))]
compile_error!("enable at least one of the v4 or v6 features");
