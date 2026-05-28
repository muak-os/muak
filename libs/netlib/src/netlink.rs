//! Abstraction over netlink operations.
//!
//! [`Ops`] is a supertrait composed of per-domain sub-traits defined
//! alongside their respective modules. [`Rtnl`] is the production
//! implementation backed by a live `rtnetlink::Handle`.

use crate::address::Ops as AddressOps;
use crate::bridge::Ops as BridgeOps;
use crate::interface::Ops as InterfaceOps;
use crate::link::Ops as LinkOps;
use crate::route::Ops as RouteOps;

/// Unified trait encapsulating all netlink I/O used by the network daemon.
pub trait Ops:
    LinkOps + AddressOps + RouteOps + BridgeOps + InterfaceOps + Clone + Send + Sync + 'static
{
}

/// Production implementation backed by `rtnetlink::Handle`.
#[derive(Clone)]
pub struct Rtnl {
    pub(crate) handle: rtnetlink::Handle,
}

impl Rtnl {
    /// Wraps an existing rtnetlink handle.
    #[must_use]
    pub fn new(handle: rtnetlink::Handle) -> Self {
        Self { handle }
    }
}

impl Ops for Rtnl {}
