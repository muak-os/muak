//! Interface and method observation vocabulary.

use alloc::vec::Vec;
use core::fmt;
use core::net::{Ipv4Addr, Ipv6Addr};

/// Address family a method runs for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Family {
    /// IPv4.
    V4,
    /// IPv6.
    V6,
}

impl fmt::Display for Family {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            Self::V4 => f.write_str("IPv4"),
            Self::V6 => f.write_str("IPv6"),
        }
    }
}

/// Provisioning lifecycle state of one address family.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MethodState {
    /// Acquiring configuration.
    Acquiring,
    /// Configuration is bound and stable.
    Bound,
    /// Running with degraded configuration.
    Degraded,
    /// Method is stopped.
    Stopped,
}

impl fmt::Display for MethodState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            Self::Acquiring => f.write_str("acquiring"),
            Self::Bound => f.write_str("bound"),
            Self::Degraded => f.write_str("degraded"),
            Self::Stopped => f.write_str("stopped"),
        }
    }
}

/// Lifecycle phase of a `DHCPv4` method.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DhcpPhase {
    /// The method has not started.
    Idle,
    /// A `DORA` exchange is in flight.
    Acquiring,
    /// The lease is held.
    Bound,
    /// The lease is being renewed.
    Renewing,
    /// The lease is being rebound.
    Rebinding,
    /// Acquisition failed or the link went down.
    Degraded,
    /// The method is stopped.
    Stopped,
}

impl fmt::Display for DhcpPhase {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            Self::Idle => f.write_str("idle"),
            Self::Acquiring => f.write_str("acquiring"),
            Self::Bound => f.write_str("bound"),
            Self::Renewing => f.write_str("renewing"),
            Self::Rebinding => f.write_str("rebinding"),
            Self::Degraded => f.write_str("degraded"),
            Self::Stopped => f.write_str("stopped"),
        }
    }
}

/// Lease details of a bound `DHCPv4` method.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DhcpLease {
    /// Address assigned to the interface.
    pub address: Ipv4Addr,
    /// Prefix length of the assigned subnet.
    pub prefix_len: u8,
    /// Server that granted the lease.
    pub server: Ipv4Addr,
    /// Default gateway, when the server advertised one.
    pub gateway: Option<Ipv4Addr>,
    /// DNS servers advertised by the server.
    pub dns: Vec<Ipv4Addr>,
    /// Lifecycle phase of the method.
    pub phase: DhcpPhase,
}

/// Addresses, routers, and DNS servers a SLAAC method learned.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SlaacUpdate {
    /// Address and prefix length pairs the method configured.
    pub addresses: Vec<(Ipv6Addr, u8)>,
    /// Known router addresses.
    pub routers: Vec<Ipv6Addr>,
    /// Learned DNS servers.
    pub dns: Vec<Ipv6Addr>,
}

#[cfg(test)]
mod tests {
    use super::{DhcpLease, DhcpPhase, Family, MethodState, SlaacUpdate};

    #[test]
    fn family_display_names_the_protocol() {
        // ACT / ASSERT
        assert_eq!(Family::V4.to_string(), "IPv4");
        assert_eq!(Family::V6.to_string(), "IPv6");
    }

    #[test]
    fn method_state_display_is_lowercase() {
        // ACT / ASSERT
        assert_eq!(MethodState::Acquiring.to_string(), "acquiring");
        assert_eq!(MethodState::Bound.to_string(), "bound");
        assert_eq!(MethodState::Degraded.to_string(), "degraded");
        assert_eq!(MethodState::Stopped.to_string(), "stopped");
    }

    #[test]
    fn dhcp_phase_display_is_lowercase() {
        // ACT / ASSERT
        assert_eq!(DhcpPhase::Idle.to_string(), "idle");
        assert_eq!(DhcpPhase::Acquiring.to_string(), "acquiring");
        assert_eq!(DhcpPhase::Bound.to_string(), "bound");
        assert_eq!(DhcpPhase::Renewing.to_string(), "renewing");
        assert_eq!(DhcpPhase::Rebinding.to_string(), "rebinding");
        assert_eq!(DhcpPhase::Degraded.to_string(), "degraded");
        assert_eq!(DhcpPhase::Stopped.to_string(), "stopped");
    }

    #[test]
    fn dhcp_lease_keeps_its_fields() {
        // ARRANGE
        let address = core::net::Ipv4Addr::new(10, 0, 0, 2);
        let server = core::net::Ipv4Addr::new(10, 0, 0, 1);
        let dns = vec![core::net::Ipv4Addr::new(8, 8, 8, 8)];

        // ACT
        let lease = DhcpLease {
            address,
            prefix_len: 24,
            server,
            gateway: Some(server),
            dns: dns.clone(),
            phase: DhcpPhase::Bound,
        };

        // ASSERT
        assert_eq!(lease.address, address);
        assert_eq!(lease.prefix_len, 24);
        assert_eq!(lease.server, server);
        assert_eq!(lease.gateway, Some(server));
        assert_eq!(lease.dns, dns);
        assert_eq!(lease.phase, DhcpPhase::Bound);
    }

    #[test]
    fn slaac_update_keeps_its_fields() {
        // ARRANGE
        let address = core::net::Ipv6Addr::LOCALHOST;
        let router = core::net::Ipv6Addr::UNSPECIFIED;

        // ACT
        let update = SlaacUpdate {
            addresses: vec![(address, 64)],
            routers: vec![router],
            dns: Vec::new(),
        };

        // ASSERT
        assert_eq!(update.addresses, vec![(address, 64)]);
        assert_eq!(update.routers, vec![router]);
        assert_eq!(update.dns, Vec::<core::net::Ipv6Addr>::new());
    }
}
