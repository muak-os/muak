use std::net::{Ipv4Addr, Ipv6Addr};
use std::sync::Arc;
use std::time::{Duration, SystemTime};

#[derive(Debug, Clone, PartialEq)]
pub enum NetworkStateKind {
    Uninitialized,
    Initializing,
    Operational, // bridge not yet ready
    Ready,
    Degraded,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ConnectivityStatus {
    Unknown,
    Checking,
    Connected,
    Disconnected,
}

#[derive(Debug, Clone)]
pub struct ConnectivityResult {
    pub status: ConnectivityStatus,
    pub dns_ok: bool,
    pub https_ok: bool,
    pub last_check: SystemTime,
    pub latency_ms: Option<u64>,
}

impl Default for ConnectivityResult {
    fn default() -> Self {
        Self {
            status: ConnectivityStatus::Unknown,
            dns_ok: false,
            https_ok: false,
            last_check: SystemTime::UNIX_EPOCH,
            latency_ms: None,
        }
    }
}

#[derive(Debug, Clone)]
pub struct IpConfig {
    pub address: Ipv4Addr,
    pub prefix_len: u8,
    pub gateway: Option<Ipv4Addr>,
    pub dns: Vec<Ipv4Addr>,
}

#[derive(Debug, Clone)]
pub struct Ipv6Config {
    pub address: Ipv6Addr,
    pub prefix_len: u8,
    pub gateway: Option<Ipv6Addr>,
    pub dns: Vec<Ipv6Addr>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum LinkStateKind {
    Up,
    NoCarrier,
    Down,
}

impl LinkStateKind {
    pub fn has_carrier(&self) -> bool {
        *self == LinkStateKind::Up
    }
}

impl std::fmt::Display for LinkStateKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LinkStateKind::Up => write!(f, "up"),
            LinkStateKind::NoCarrier => write!(f, "no-carrier"),
            LinkStateKind::Down => write!(f, "down"),
        }
    }
}

#[derive(Debug, Clone)]
pub struct DhcpLease {
    pub obtained_at: SystemTime,
    pub lease_time: Duration,
    pub renewal_time: Duration,
    pub rebind_time: Duration,
}

impl DhcpLease {
    pub fn expiry(&self) -> SystemTime {
        self.obtained_at + self.lease_time
    }
}

#[derive(Debug, Clone)]
pub struct InterfaceSnapshot {
    pub name: String,
    pub index: u32,
    pub mac: [u8; 6],
    pub link: LinkStateKind,
    pub ip: Option<IpConfig>,
    pub lease: Option<DhcpLease>,
    pub ipv6: Option<Ipv6Config>,
}

#[derive(Debug, Clone)]
pub struct NetworkSnapshot {
    pub state: NetworkStateKind,
    pub connectivity: ConnectivityResult,
    pub primary: Option<String>,
    pub backups: Vec<String>,
    pub interfaces: Vec<Arc<InterfaceSnapshot>>,
    pub ipv6: bool,
}

impl NetworkSnapshot {
    pub fn empty() -> Self {
        Self {
            state: NetworkStateKind::Uninitialized,
            connectivity: ConnectivityResult::default(),
            primary: None,
            backups: Vec::new(),
            interfaces: Vec::new(),
            ipv6: false,
        }
    }
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, SystemTime};

    use super::*;

    #[test]
    fn dhcp_lease_expiry_is_obtained_plus_lease_time() {
        // ARRANGE
        let obtained = SystemTime::UNIX_EPOCH + Duration::from_secs(1000);
        let lease = DhcpLease {
            obtained_at: obtained,
            lease_time: Duration::from_secs(3600),
            renewal_time: Duration::from_secs(1800),
            rebind_time: Duration::from_secs(3150),
        };
        let expected = obtained + Duration::from_secs(3600);

        // ACT
        let result = lease.expiry();

        // ASSERT
        assert_eq!(result, expected);
    }

    #[test]
    fn dhcp_lease_expiry_at_epoch() {
        // ARRANGE
        let lease = DhcpLease {
            obtained_at: SystemTime::UNIX_EPOCH,
            lease_time: Duration::from_secs(86400),
            renewal_time: Duration::from_secs(43200),
            rebind_time: Duration::from_secs(75600),
        };
        let expected = SystemTime::UNIX_EPOCH + Duration::from_secs(86400);

        // ACT
        let result = lease.expiry();

        // ASSERT
        assert_eq!(result, expected);
    }

    #[test]
    fn connectivity_result_default() {
        // ACT
        let r = ConnectivityResult::default();

        // ASSERT
        assert_eq!(r.status, ConnectivityStatus::Unknown);
        assert!(!r.dns_ok);
        assert!(!r.https_ok);
        assert_eq!(r.last_check, SystemTime::UNIX_EPOCH);
        assert!(r.latency_ms.is_none());
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
        assert!(!snap.ipv6);
        assert_eq!(snap.connectivity.status, ConnectivityStatus::Unknown);
    }
}
