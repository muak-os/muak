//! Abstraction over netlink operations.
//!
//! [`NetlinkOps`] is a supertrait composed of per-domain sub-traits defined
//! alongside their respective modules. [`RtnetlinkOps`] is the production
//! implementation backed by a live `rtnetlink::Handle`.

pub use crate::address::AddressOps;
pub use crate::bridge::BridgeOps;
pub use crate::interface::InterfaceOps;
pub use crate::link::LinkOps;
pub use crate::route::RouteOps;

/// Unified trait encapsulating all netlink I/O used by the network daemon.
pub trait NetlinkOps:
    LinkOps + AddressOps + RouteOps + BridgeOps + InterfaceOps + Clone + Send + Sync + 'static
{
}

/// Production implementation backed by `rtnetlink::Handle`.
#[derive(Clone)]
pub struct RtnetlinkOps {
    pub(crate) handle: rtnetlink::Handle,
}

impl RtnetlinkOps {
    /// Wraps an existing rtnetlink handle.
    pub fn new(handle: rtnetlink::Handle) -> Self {
        Self { handle }
    }
}

impl NetlinkOps for RtnetlinkOps {}
