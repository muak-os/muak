//! Per-interface network configuration finite state machine.

#![warn(missing_docs)]

extern crate alloc;

mod adapter;
mod controller;
mod method;

use alloc::collections::VecDeque;
use alloc::string::String;
use alloc::vec::Vec;
use core::net::{IpAddr, Ipv4Addr, Ipv6Addr};

/// Address family the controller drives for one interface.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Family {
    /// IPv4.
    V4,
    /// IPv6.
    V6,
}

/// Commands the supervisor sends to an interface controller.
#[derive(Debug, Clone)]
pub enum Command {
    /// Apply the given per-family configurations.
    Configure {
        /// IPv4 configuration to apply.
        v4: Option<netmodel::Ipv4Config>,
        /// IPv6 configuration to apply.
        v6: Option<netmodel::Ipv6Config>,
    },
    /// Reconcile desired and actual configuration.
    Reconcile,
    /// Bring the link administratively up.
    LinkUp,
    /// Bring the link administratively down.
    LinkDown,
    /// Stop every method and shut the controller down.
    Shutdown,
}

/// Input an interface controller consumes on each step.
#[derive(Debug)]
pub enum Input<'a> {
    /// Poll the controller without new external input.
    Poll,
    /// Supervisor command.
    Command(Command),
    /// Link snapshot update.
    Link(ip_method::LinkInfo),
    /// Input for one address family method.
    Method {
        /// Family the input belongs to.
        family: Family,
        /// Input to hand to the family method.
        input: ip_method::Input<'a>,
    },
    /// Timer deadline elapsed.
    Timer,
}

/// Effect an interface controller requests from the runtime.
#[derive(Debug)]
pub enum Effect<'a> {
    /// Bring the link administratively up.
    SetLinkUp {
        /// Link to bring up.
        link: netmodel::Name,
    },
    /// Bring the link administratively down.
    SetLinkDown {
        /// Link to bring down.
        link: netmodel::Name,
    },
    /// Assign an address to the link.
    AddAddress {
        /// Link to assign the address to.
        link: netmodel::Name,
        /// Address to assign.
        address: IpAddr,
        /// Prefix length in bits.
        prefix_len: u8,
    },
    /// Remove an address from the link.
    RemoveAddress {
        /// Link to remove the address from.
        link: netmodel::Name,
        /// Address to remove.
        address: IpAddr,
        /// Prefix length in bits, the address port needs the exact pair.
        prefix_len: u8,
    },
    /// Ensure a default route through the gateway.
    EnsureRoute {
        /// Link the route goes through.
        link: netmodel::Name,
        /// Gateway of the default route.
        gateway: IpAddr,
    },
    /// Remove the default route through the gateway.
    RemoveRoute {
        /// Link the route goes through.
        link: netmodel::Name,
        /// Gateway of the default route.
        gateway: IpAddr,
    },
    /// Open a socket for the family.
    OpenSocket {
        /// Family the socket belongs to.
        family: Family,
        /// Specification of the socket to open.
        spec: ip_method::SocketSpec,
    },
    /// Send a frame on a socket the family already opened.
    ///
    /// `dst` is `None` for raw layer 2 and `UDP` sockets, whose destination
    /// comes from the frame and the socket peer, and required for raw
    /// `ICMPv6` sockets.
    Send {
        /// Family the socket belongs to.
        family: Family,
        /// Slot of the socket to send on.
        slot: ip_method::SocketSlot,
        /// Raw frame bytes to send.
        frame: &'a [u8],
        /// Destination address, required only for `ICMPv6` sockets.
        dst: Option<core::net::SocketAddr>,
    },
    /// Close a socket the family opened.
    CloseSocket {
        /// Family the socket belongs to.
        family: Family,
        /// Slot of the socket to close.
        slot: ip_method::SocketSlot,
    },
    /// Request fresh entropy for the family method.
    RequestEntropy {
        /// Family the entropy is requested for.
        family: Family,
    },
    /// Publish the interface snapshot.
    Publish,
}

/// Outcome of one interface controller step.
#[derive(Debug)]
pub enum Step<'a> {
    /// The controller produced an effect for the runtime.
    Effect(Effect<'a>),
    /// The controller is waiting for input or for its wake deadline.
    Wait,
}

/// IPv4 method view exposed to the supervisor.
#[derive(Debug, Clone)]
pub enum V4View {
    /// Static address configuration.
    Static(ip_static::View<Ipv4Addr>),
    /// `DHCPv4` lease and lifecycle phase.
    #[cfg(feature = "ipv4")]
    Dhcp(dhcp::View),
}

/// IPv6 method view exposed to the supervisor.
#[derive(Debug, Clone)]
pub enum V6View {
    /// Static address configuration.
    Static(ip_static::View<Ipv6Addr>),
    /// Stateless autoconfiguration addresses, routers, and DNS.
    #[cfg(feature = "ipv6")]
    Slaac(slaac::View),
}

/// Point-in-time state of an interface and its address methods.
#[derive(Debug, Clone)]
pub struct Snapshot {
    /// Administrative and carrier state of the link.
    pub link: netmodel::LinkState,
    /// IPv4 method view, when configured.
    pub v4: Option<V4View>,
    /// IPv4 provisioning phase.
    pub v4_state: ip_method::State,
    /// IPv6 method view, when configured.
    pub v6: Option<V6View>,
    /// IPv6 provisioning phase.
    pub v6_state: ip_method::State,
    /// Warnings and failures reported by the family methods.
    pub notices: Vec<String>,
}

/// Family-neutral controller for one interface.
pub struct Controller {
    pub(crate) link: ip_method::LinkInfo,
    pub(crate) snapshot: Snapshot,
    pub(crate) v4: Option<method::V4Slot>,
    pub(crate) v6: Option<method::V6Slot>,
    pub(crate) pending: VecDeque<(Family, ip_method::Input<'static>)>,
    pub(crate) deferred: Option<method::Replacement>,
    pub(crate) shutdown: bool,
}
