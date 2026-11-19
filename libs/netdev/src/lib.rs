//! Domain-layer network device abstraction.

#![warn(missing_docs)]

pub mod interface;

use netlink::Rtnl;

/// Unified trait encapsulating all netlink I/O used by the network daemon.
pub trait Ops:
    netlink::link::Ops
    + netlink::address::Ops
    + netlink::route::Ops
    + netlink::bridge::Ops
    + interface::Ops
    + Clone
    + Send
    + Sync
    + 'static
{
}

impl Ops for Rtnl {}
