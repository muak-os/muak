use std::net::Ipv4Addr;
use std::time::{Duration, SystemTime};

#[derive(Debug, Clone, PartialEq)]
pub enum NetworkStateKind {
    Uninitialized,
    Initializing,
    Operational, // bridge not yet ready
    Ready,
    Degraded,
    Failed,
}

#[derive(Debug, Clone)]
pub struct IpConfig {
    pub address: Ipv4Addr,
    pub prefix_len: u8,
    pub gateway: Option<Ipv4Addr>,
    pub dns: Vec<Ipv4Addr>,
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
    pub server_id: Ipv4Addr,
    pub ip: IpConfig,
}

impl DhcpLease {
    pub fn expiry(&self) -> SystemTime {
        self.obtained_at + self.lease_time
    }
    pub fn should_renew(&self) -> bool {
        SystemTime::now() >= self.obtained_at + self.renewal_time
    }
    pub fn should_rebind(&self) -> bool {
        SystemTime::now() >= self.obtained_at + self.rebind_time
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
}

#[derive(Debug, Clone)]
pub struct NetworkSnapshot {
    pub state: NetworkStateKind,
    pub primary: Option<String>,
    pub backups: Vec<String>,
    pub interfaces: Vec<InterfaceSnapshot>,
}

impl NetworkSnapshot {
    pub fn empty() -> Self {
        Self {
            state: NetworkStateKind::Uninitialized,
            primary: None,
            backups: Vec::new(),
            interfaces: Vec::new(),
        }
    }
}
