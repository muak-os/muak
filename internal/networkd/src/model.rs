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
    Down,
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
    pub ipv6_lease: Option<DhcpLease>,
}

#[derive(Debug, Clone)]
pub struct NetworkSnapshot {
    pub state: NetworkStateKind,
    pub connectivity: ConnectivityResult,
    pub primary: Option<String>,
    pub backups: Vec<String>,
    pub interfaces: Vec<Arc<InterfaceSnapshot>>,
    pub ipv6_available: bool,
}

impl NetworkSnapshot {
    pub fn empty() -> Self {
        Self {
            state: NetworkStateKind::Uninitialized,
            connectivity: ConnectivityResult::default(),
            primary: None,
            backups: Vec::new(),
            interfaces: Vec::new(),
            ipv6_available: false,
        }
    }
}
