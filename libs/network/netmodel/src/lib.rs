//! Network domain model and configuration vocabulary.

#![warn(missing_docs)]

pub mod error;
mod interface;
mod link;
mod name;
mod selection;
mod state;

use core::net::{IpAddr, Ipv4Addr, Ipv6Addr};

use mac::Mac;

use crate::error::LinkError;

// Maximum interface name payload length in bytes, excluding the NUL terminator.
const MAX_LEN: usize = 15;

/// A validated Linux network interface name.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct Name {
    bytes: [u8; MAX_LEN],
    len: u8,
}

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

/// Address family an address method runs for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Family {
    /// IPv4.
    V4,
    /// IPv6.
    V6,
}

/// Lifecycle phase of an address method.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MethodState {
    /// Acquiring address configuration.
    Acquiring,
    /// Configuration is bound and stable.
    Bound,
    /// Running with degraded configuration.
    Degraded,
    /// Method is stopped.
    Stopped,
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

/// Network configuration covering DNS, IPv6 toggle, and interfaces.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Config {
    /// Whether IPv6 networking is enabled.
    pub ipv6: bool,
    /// DNS server addresses for name resolution.
    pub dns: Vec<IpAddr>,
    /// Declarative list of network interfaces.
    pub interfaces: Vec<InterfaceConfig>,
}

/// Declarative configuration for a single network interface.
#[derive(Debug, Clone, PartialEq)]
pub struct InterfaceConfig {
    /// Interface name.
    pub name: String,
    /// Interface kind.
    pub kind: InterfaceKind,
    /// Optional IPv4 configuration.
    pub ipv4: Option<Ipv4Config>,
    /// Optional IPv6 configuration.
    pub ipv6: Option<Ipv6Config>,
    /// Optional bridge-specific configuration.
    pub bridge: Option<BridgeConfig>,
}

/// Kind of network interface.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InterfaceKind {
    /// Bridge interface connecting multiple networks.
    Bridge,
    /// Standard Ethernet interface.
    Ethernet,
}

/// Bridge-specific configuration.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct BridgeConfig {
    /// Member ports attached to the bridge.
    pub port: Vec<String>,
    /// Whether Spanning Tree Protocol is enabled.
    pub stp: bool,
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
    Error(LinkError),
}
