//! Point-in-time interface and address-method state.

#[cfg(feature = "ipv6")]
use alloc::vec::Vec;
#[cfg(feature = "ipv6")]
use core::net::Ipv6Addr;

use netlink::link::state::State;

/// Point-in-time view of an interface and its address methods.
#[derive(Debug, Clone)]
pub struct Snapshot {
    /// Administrative and carrier state of the link.
    pub link: State,
    /// IPv4 method state, when enabled and configured.
    #[cfg(feature = "ipv4")]
    pub v4: Option<V4State>,
    /// IPv6 method state, when enabled and configured.
    #[cfg(feature = "ipv6")]
    pub v6: Option<V6State>,
}

/// `DHCPv4` phase tracked by the v4 method.
#[cfg(feature = "ipv4")]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DhcpState {
    /// Discovering a server.
    Init,
    /// Selecting among offers.
    Selecting,
    /// Requesting the selected lease.
    Requesting,
    /// Lease acquired.
    Bound,
    /// Renewing the lease.
    Renewing,
    /// Rebinding the lease.
    Rebinding,
}

/// IPv4 configuration source tracked by the v4 method.
#[cfg(feature = "ipv4")]
#[derive(Debug, Clone)]
pub enum V4State {
    /// Static address configuration.
    Static(netlink::address::IpConfig),
    /// `DHCPv4` lease and phase.
    Dhcp {
        /// Current lease.
        lease: dhcp_client::lease::Lease,
        /// Current DHCP phase.
        phase: DhcpState,
    },
}

/// IPv6 configuration source tracked by the v6 method.
#[cfg(feature = "ipv6")]
#[derive(Debug, Clone)]
pub enum V6State {
    /// Static address configuration.
    Static(netlink::address::Ipv6Config),
    /// SLAAC-derived state.
    Slaac {
        /// Address and prefix length pairs.
        addresses: Vec<(Ipv6Addr, u8)>,
        /// Known router addresses.
        routers: Vec<Ipv6Addr>,
        /// Learned DNS servers.
        dns: Vec<Ipv6Addr>,
    },
}
