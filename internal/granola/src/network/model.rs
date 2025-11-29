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

#[derive(Debug, Clone)]
pub struct Ipv4Config {
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

// Legacy alias for backward compatibility during transition
pub type IpConfig = Ipv4Config;

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
    /// IPv4 configuration (if acquired)
    pub ipv4: Option<Ipv4Config>,
    /// IPv4 DHCP lease (if acquired via DHCP)
    pub ipv4_lease: Option<DhcpLease>,
    /// IPv6 configuration (if acquired)
    pub ipv6: Option<Ipv6Config>,
    /// IPv6 DHCP lease (if acquired via DHCPv6)
    pub ipv6_lease: Option<DhcpLease>,
    
    // Legacy fields for backward compatibility during transition
    #[deprecated(note = "Use ipv4 field instead")]
    pub ip: Option<Ipv4Config>,
    #[deprecated(note = "Use ipv4_lease field instead")]
    pub lease: Option<DhcpLease>,
}

#[derive(Debug, Clone)]
pub struct NetworkSnapshot {
    pub state: NetworkStateKind,
    /// Primary (preferred) interface - the interface we want to use (from configuration)
    pub primary: Option<String>,
    /// Active interface - the interface currently carrying traffic (runtime state)
    pub active: Option<String>,
    /// Secondary (backup) interfaces available for failover
    pub secondaries: Vec<String>,
    pub interfaces: Vec<Arc<InterfaceSnapshot>>,
    /// Whether IPv6 connectivity is available (DHCPv6 succeeded)
    pub ipv6_available: bool,
}

impl NetworkSnapshot {
    pub fn empty() -> Self {
        Self {
            state: NetworkStateKind::Uninitialized,
            primary: None,
            active: None,
            secondaries: Vec::new(),
            interfaces: Vec::new(),
            ipv6_available: false,
        }
    }
    
    /// Check if we're running on the preferred interface (optimal state)
    pub fn is_on_primary(&self) -> bool {
        match (&self.primary, &self.active) {
            (Some(p), Some(a)) => p == a,
            _ => false,
        }
    }
}
