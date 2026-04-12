//! Pure data types for interface and network-level snapshots.

use std::net::{Ipv4Addr, Ipv6Addr};
use std::sync::Arc;

use anyhow::Result;
use netlib::address::{IpConfig, Ipv6Config};
use netlib::interface::InterfaceName;
use netlib::link::LinkStateKind;

use crate::dhcp::{DhcpLease, DhcpState};
use crate::dns;
use crate::state_machine::StateMachine;

/// Life cycle state of a single network interface.
#[derive(Debug, Clone, PartialEq)]
pub enum InterfaceState {
    Discovered,
    Configuring,
    Configured,
    Degraded,
    Failed,
    Deconfiguring,
}

impl StateMachine for InterfaceState {
    fn valid_next_states(&self) -> &'static [Self] {
        match self {
            Self::Discovered => &[Self::Configuring],
            Self::Configuring => &[Self::Configured, Self::Failed],
            Self::Configured => &[Self::Degraded, Self::Deconfiguring],
            Self::Degraded => &[Self::Configuring, Self::Configured, Self::Deconfiguring],
            Self::Failed => &[Self::Configuring, Self::Deconfiguring],
            Self::Deconfiguring => &[Self::Discovered],
        }
    }
}

impl std::fmt::Display for InterfaceState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Discovered => f.write_str("Discovered"),
            Self::Configuring => f.write_str("Configuring"),
            Self::Configured => f.write_str("Configured"),
            Self::Degraded => f.write_str("Degraded"),
            Self::Failed => f.write_str("Failed"),
            Self::Deconfiguring => f.write_str("Deconfiguring"),
        }
    }
}

/// Global network readiness derived from aggregated interface states.
#[derive(Debug, Clone, PartialEq)]
pub enum NetworkStateKind {
    Uninitialized,
    Initializing,
    Operational,
    Ready,
    Degraded,
}

impl StateMachine for NetworkStateKind {
    fn valid_next_states(&self) -> &'static [Self] {
        match self {
            Self::Uninitialized => &[Self::Initializing],
            Self::Initializing => &[Self::Operational, Self::Degraded],
            Self::Operational => &[Self::Ready, Self::Degraded],
            Self::Ready => &[Self::Degraded],
            Self::Degraded => &[Self::Initializing, Self::Operational],
        }
    }
}

impl std::fmt::Display for NetworkStateKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Uninitialized => f.write_str("Uninitialized"),
            Self::Initializing => f.write_str("Initializing"),
            Self::Operational => f.write_str("Operational"),
            Self::Ready => f.write_str("Ready"),
            Self::Degraded => f.write_str("Degraded"),
        }
    }
}

#[derive(Debug, Clone)]
pub struct InterfaceSnapshot {
    pub name: InterfaceName,
    pub state: InterfaceState,
    pub index: u32,
    pub mac: [u8; 6],
    pub link: LinkStateKind,
    pub ip: Option<IpConfig>,
    pub lease: Option<DhcpLease>,
    pub dhcp_state: Option<DhcpState>,
    pub ipv6: Option<Ipv6Config>,
}

impl InterfaceSnapshot {
    /// Advances this interface's life cycle state, logging and validating the transition.
    pub fn transition(&mut self, to: InterfaceState) -> Result<()> {
        let from = self.state.clone();
        self.state
            .transition(to)
            .map_err(|e| anyhow::anyhow!("{e}"))?;
        kmsg::info!("Interface {} state: {} -> {}", self.name, from, self.state);
        Ok(())
    }
}

/// Tracks all known DNS nameservers across v4 and v6.
#[derive(Debug, Clone, Default)]
pub struct DnsState {
    pub v4: Vec<Ipv4Addr>,
    pub v6: Vec<Ipv6Addr>,
}

impl DnsState {
    /// Flushes the current state to resolv.conf via atomic write.
    pub fn flush(&self) -> Result<()> {
        dns::write_resolv_conf(&self.v4, &self.v6)
    }
}

#[derive(Debug, Clone)]
pub struct NetworkSnapshot {
    pub state: NetworkStateKind,
    pub primary: Option<InterfaceName>,
    pub backups: Vec<InterfaceName>,
    pub interfaces: Vec<Arc<InterfaceSnapshot>>,
}

impl NetworkSnapshot {
    /// Returns an empty snapshot in the uninitialized state.
    pub fn empty() -> Self {
        Self {
            state: NetworkStateKind::Uninitialized,
            primary: None,
            backups: Vec::new(),
            interfaces: Vec::new(),
        }
    }

    /// Advances the network state, logging and validating the transition.
    pub fn transition(&mut self, to: NetworkStateKind) -> Result<()> {
        let from = self.state.clone();
        self.state
            .transition(to)
            .map_err(|e| anyhow::anyhow!("{e}"))?;
        kmsg::info!("Network state: {} -> {}", from, self.state);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, SystemTime};

    use super::*;

    fn make_lease() -> DhcpLease {
        DhcpLease {
            obtained_at: SystemTime::UNIX_EPOCH + Duration::from_secs(1000),
            lease_time: Duration::from_secs(3600),
            renewal_time: Duration::from_secs(1800),
            rebind_time: Duration::from_secs(3150),
            server_ip: Ipv4Addr::new(192, 168, 1, 1),
            assigned_ip: Ipv4Addr::new(192, 168, 1, 100),
            prefix_len: 24,
            gateway: Some(Ipv4Addr::new(192, 168, 1, 1)),
            dns_servers: vec![Ipv4Addr::new(8, 8, 8, 8)],
        }
    }

    #[test]
    fn dhcp_lease_expiry_is_obtained_plus_lease_time() {
        // ARRANGE
        let lease = make_lease();
        let expected = lease.obtained_at + lease.lease_time;

        // ACT
        let result = lease.expiry();

        // ASSERT
        assert_eq!(result, expected);
    }

    #[test]
    fn dhcp_lease_expiry_at_epoch() {
        // ARRANGE
        let mut lease = make_lease();
        lease.obtained_at = SystemTime::UNIX_EPOCH;
        lease.lease_time = Duration::from_secs(86400);
        let expected = SystemTime::UNIX_EPOCH + Duration::from_secs(86400);

        // ACT
        let result = lease.expiry();

        // ASSERT
        assert_eq!(result, expected);
    }

    #[test]
    fn network_snapshot_empty_defaults() {
        // ACT
        let snap = NetworkSnapshot::empty();

        // ASSERT
        assert_eq!(snap.state, NetworkStateKind::Uninitialized);
        assert!(snap.primary.is_none());
        assert!(snap.backups.is_empty());
        assert!(snap.interfaces.is_empty());
    }

    #[test]
    fn dns_state_default_is_empty() {
        // ACT
        let dns = DnsState::default();

        // ASSERT
        assert!(dns.v4.is_empty());
        assert!(dns.v6.is_empty());
    }

    #[test]
    fn network_state_kind_display() {
        // ACT / ASSERT
        assert_eq!(NetworkStateKind::Uninitialized.to_string(), "Uninitialized");
        assert_eq!(NetworkStateKind::Initializing.to_string(), "Initializing");
        assert_eq!(NetworkStateKind::Operational.to_string(), "Operational");
        assert_eq!(NetworkStateKind::Ready.to_string(), "Ready");
        assert_eq!(NetworkStateKind::Degraded.to_string(), "Degraded");
    }

    #[test]
    fn valid_transitions_succeed() {
        // ARRANGE
        let pairs = [
            (
                NetworkStateKind::Uninitialized,
                NetworkStateKind::Initializing,
            ),
            (
                NetworkStateKind::Initializing,
                NetworkStateKind::Operational,
            ),
            (NetworkStateKind::Initializing, NetworkStateKind::Degraded),
            (NetworkStateKind::Operational, NetworkStateKind::Ready),
            (NetworkStateKind::Operational, NetworkStateKind::Degraded),
            (NetworkStateKind::Ready, NetworkStateKind::Degraded),
            (NetworkStateKind::Degraded, NetworkStateKind::Initializing),
            (NetworkStateKind::Degraded, NetworkStateKind::Operational),
        ];

        for (from, to) in pairs {
            // ARRANGE
            let mut snap = NetworkSnapshot::empty();
            snap.state = from.clone();

            // ACT
            let result = snap.transition(to.clone());

            // ASSERT
            assert!(result.is_ok(), "{from} -> {to} should be valid");
            assert_eq!(snap.state, to);
        }
    }

    #[test]
    fn invalid_transitions_return_error() {
        // ARRANGE
        let pairs = [
            (
                NetworkStateKind::Uninitialized,
                NetworkStateKind::Operational,
            ),
            (NetworkStateKind::Uninitialized, NetworkStateKind::Ready),
            (NetworkStateKind::Uninitialized, NetworkStateKind::Degraded),
            (NetworkStateKind::Initializing, NetworkStateKind::Ready),
            (
                NetworkStateKind::Initializing,
                NetworkStateKind::Uninitialized,
            ),
            (
                NetworkStateKind::Operational,
                NetworkStateKind::Uninitialized,
            ),
            (
                NetworkStateKind::Operational,
                NetworkStateKind::Initializing,
            ),
            (NetworkStateKind::Ready, NetworkStateKind::Operational),
            (NetworkStateKind::Ready, NetworkStateKind::Uninitialized),
            (NetworkStateKind::Ready, NetworkStateKind::Initializing),
            (NetworkStateKind::Degraded, NetworkStateKind::Ready),
            (NetworkStateKind::Degraded, NetworkStateKind::Uninitialized),
        ];

        for (from, to) in pairs {
            // ARRANGE
            let mut snap = NetworkSnapshot::empty();
            snap.state = from.clone();

            // ACT
            let result = snap.transition(to.clone());

            // ASSERT
            assert!(result.is_err(), "{from} -> {to} should be invalid");
            assert_eq!(
                snap.state, from,
                "state must not change on invalid transition"
            );
        }
    }

    #[test]
    fn transition_does_not_mutate_state_on_error() {
        // ARRANGE
        let mut snap = NetworkSnapshot::empty();
        snap.state = NetworkStateKind::Ready;

        // ACT
        let _ = snap.transition(NetworkStateKind::Initializing);

        // ASSERT
        assert_eq!(snap.state, NetworkStateKind::Ready);
    }

    fn make_interface_snapshot(state: InterfaceState) -> InterfaceSnapshot {
        InterfaceSnapshot {
            name: InterfaceName::new("eth0").expect("valid name"),
            state,
            index: 2,
            mac: [0x00, 0x11, 0x22, 0x33, 0x44, 0x55],
            link: netlib::link::LinkStateKind::Up,
            ip: None,
            lease: None,
            dhcp_state: None,
            ipv6: None,
        }
    }

    #[test]
    fn interface_state_display() {
        // ACT / ASSERT
        assert_eq!(InterfaceState::Discovered.to_string(), "Discovered");
        assert_eq!(InterfaceState::Configuring.to_string(), "Configuring");
        assert_eq!(InterfaceState::Configured.to_string(), "Configured");
        assert_eq!(InterfaceState::Degraded.to_string(), "Degraded");
        assert_eq!(InterfaceState::Failed.to_string(), "Failed");
        assert_eq!(InterfaceState::Deconfiguring.to_string(), "Deconfiguring");
    }

    #[test]
    fn valid_interface_transitions_succeed() {
        // ARRANGE
        let pairs = [
            (InterfaceState::Discovered, InterfaceState::Configuring),
            (InterfaceState::Configuring, InterfaceState::Configured),
            (InterfaceState::Configuring, InterfaceState::Failed),
            (InterfaceState::Configured, InterfaceState::Degraded),
            (InterfaceState::Configured, InterfaceState::Deconfiguring),
            (InterfaceState::Degraded, InterfaceState::Configuring),
            (InterfaceState::Degraded, InterfaceState::Configured),
            (InterfaceState::Degraded, InterfaceState::Deconfiguring),
            (InterfaceState::Failed, InterfaceState::Configuring),
            (InterfaceState::Failed, InterfaceState::Deconfiguring),
            (InterfaceState::Deconfiguring, InterfaceState::Discovered),
        ];

        for (from, to) in pairs {
            // ARRANGE
            let mut snap = make_interface_snapshot(from.clone());

            // ACT
            let result = snap.transition(to.clone());

            // ASSERT
            assert!(result.is_ok(), "{from} -> {to} should be valid");
            assert_eq!(snap.state, to);
        }
    }

    #[test]
    fn invalid_interface_transitions_return_error() {
        // ARRANGE
        let pairs = [
            (InterfaceState::Discovered, InterfaceState::Configured),
            (InterfaceState::Discovered, InterfaceState::Degraded),
            (InterfaceState::Discovered, InterfaceState::Failed),
            (InterfaceState::Discovered, InterfaceState::Deconfiguring),
            (InterfaceState::Configuring, InterfaceState::Discovered),
            (InterfaceState::Configuring, InterfaceState::Degraded),
            (InterfaceState::Configuring, InterfaceState::Deconfiguring),
            (InterfaceState::Configured, InterfaceState::Discovered),
            (InterfaceState::Configured, InterfaceState::Configuring),
            (InterfaceState::Configured, InterfaceState::Failed),
            (InterfaceState::Degraded, InterfaceState::Discovered),
            (InterfaceState::Degraded, InterfaceState::Failed),
            (InterfaceState::Failed, InterfaceState::Discovered),
            (InterfaceState::Failed, InterfaceState::Configured),
            (InterfaceState::Failed, InterfaceState::Degraded),
            (InterfaceState::Deconfiguring, InterfaceState::Configured),
            (InterfaceState::Deconfiguring, InterfaceState::Configuring),
            (InterfaceState::Deconfiguring, InterfaceState::Degraded),
            (InterfaceState::Deconfiguring, InterfaceState::Failed),
        ];

        for (from, to) in pairs {
            // ARRANGE
            let mut snap = make_interface_snapshot(from.clone());

            // ACT
            let result = snap.transition(to.clone());

            // ASSERT
            assert!(result.is_err(), "{from} -> {to} should be invalid");
            assert_eq!(
                snap.state, from,
                "state must not change on invalid transition"
            );
        }
    }

    #[test]
    fn interface_transition_does_not_mutate_state_on_error() {
        // ARRANGE
        let mut snap = make_interface_snapshot(InterfaceState::Configured);

        // ACT
        let _ = snap.transition(InterfaceState::Discovered);

        // ASSERT
        assert_eq!(snap.state, InterfaceState::Configured);
    }

    #[test]
    fn interface_snapshot_starts_discovered_after_construction() {
        // ACT
        let snap = make_interface_snapshot(InterfaceState::Discovered);

        // ASSERT
        assert_eq!(snap.state, InterfaceState::Discovered);
    }
}
