//! Network domain model and configuration vocabulary.

#![warn(missing_docs)]

mod address;
mod event;
mod hardware;
mod link;
mod name;
mod selection;
mod state;

use core::net::{IpAddr, Ipv4Addr, Ipv6Addr};
use core::num::NonZeroI32;

use thiserror::Error;

/// A validated Linux network interface name.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Name(String);

/// An invalid network interface name.
#[derive(Debug, Error)]
#[error("invalid interface name: {0:?}")]
pub struct InvalidName(String);

/// A 6-byte hardware address.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct Mac([u8; 6]);

/// Administrative state of a network link.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum AdminState {
    /// Administratively enabled.
    Up,
    /// Administratively disabled.
    #[default]
    Down,
}

/// Carrier state of a network link as reported by the kernel.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum CarrierState {
    /// Carrier detected.
    Up,
    /// Carrier down.
    Down,
    /// Carrier dormant, waiting for an external event.
    Dormant,
    /// The driver does not report a carrier state.
    #[default]
    Unknown,
}

/// Administrative and carrier state of a network link.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LinkState {
    /// Link is administratively up and the carrier is usable.
    Up,
    /// Link is administratively up but the carrier is down.
    NoCarrier,
    /// Link is administratively up and the carrier is dormant, waiting for an external event.
    Dormant,
    /// Link is administratively down, so its carrier is not observable. This is not a no-carrier verdict.
    Down,
}

/// A domain snapshot of a network link.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Link {
    /// Interface name.
    pub name: Name,
    /// Kernel interface index.
    pub index: u32,
    /// Hardware address.
    pub mac: Mac,
    /// Administrative state.
    pub admin: AdminState,
    /// Carrier state.
    pub carrier: CarrierState,
    /// Whether the device carries a virtual link kind.
    pub is_virtual: bool,
    /// Index of the master (bridge/bond) this device is enslaved to, if any.
    pub master: Option<u32>,
}

/// An IP address with its prefix length.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Address {
    /// Assigned IP address.
    pub address: IpAddr,
    /// Prefix length in bits.
    pub prefix_len: u8,
}

/// A route destination with an optional gateway.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Route {
    /// Destination IP address.
    pub destination: IpAddr,
    /// Prefix length in bits.
    pub prefix_len: u8,
    /// Optional gateway for the route.
    pub gateway: Option<IpAddr>,
}

/// IPv4 address provisioning configuration.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Ipv4Config {
    /// Address and prefix length pairs.
    pub addresses: Vec<(Ipv4Addr, u8)>,
    /// Optional default gateway.
    pub gateway: Option<Ipv4Addr>,
    /// DNS server addresses.
    pub dns: Vec<Ipv4Addr>,
    /// Whether the interface acquires its address over `DHCPv4`.
    pub dhcp: bool,
}

/// IPv6 address provisioning configuration.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Ipv6Config {
    /// Address and prefix length pairs.
    pub addresses: Vec<(Ipv6Addr, u8)>,
    /// Optional default gateway.
    pub gateway: Option<Ipv6Addr>,
    /// DNS server addresses.
    pub dns: Vec<Ipv6Addr>,
    /// Whether stateless autoconfiguration is enabled.
    pub autoconf: bool,
}

/// Link selection logic for choosing the best primary and backup links.
pub struct Selector;

/// A link lifecycle event.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LinkEvent {
    /// Carrier detected on the link.
    Up {
        /// Interface name.
        name: Name,
        /// Kernel interface index.
        index: u32,
    },
    /// Carrier lost on the link.
    Down {
        /// Interface name.
        name: Name,
        /// Kernel interface index.
        index: u32,
    },
    /// The link was added.
    Added {
        /// Interface name.
        name: Name,
        /// Kernel interface index.
        index: u32,
        /// Hardware address.
        mac: Mac,
    },
    /// The link was removed.
    Deleted {
        /// Interface name.
        name: Name,
        /// Kernel interface index.
        index: u32,
    },
    /// Link monitoring reported a failure.
    Error(LinkFailure),
}

/// A failure reported by link monitoring.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum LinkFailure {
    /// Netlink notification buffer overran, so link events may have been lost.
    #[error("netlink notification buffer overran, events may be lost")]
    Overrun,
    /// Netlink reported an error on the event socket.
    #[error("netlink event socket error: {0}")]
    Kernel(NonZeroI32),
    /// Netlink event stream closed, so link events are no longer delivered.
    #[error("netlink event stream closed")]
    Closed,
}
