//! Family-neutral per-interface address configuration controller.

#![warn(missing_docs)]

extern crate alloc;

pub mod disabled;
pub mod family;
pub mod interface;
pub mod system;
pub mod transport;
pub mod view;

/// Interface controller driving the address families this build enables.
pub type System<N> = interface::Interface<N, system::V4, system::V6>;
