//! Networking primitives library.

#![warn(missing_docs)]

extern crate alloc;

pub mod address;
pub mod bridge;
pub mod interface;
pub mod link;
pub mod mac;
pub mod monitor;
pub mod netlink;
pub mod packet;
pub mod retry;
pub mod route;
pub mod slaac;
pub mod socket;
pub mod tap;
