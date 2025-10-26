use std::net::Ipv4Addr;

// Bridge configuration
pub const BRIDGE_NAME: &str = "muak0";
pub const BRIDGE_IP: Ipv4Addr = Ipv4Addr::new(10, 42, 0, 1);
pub const BRIDGE_PREFIX_LEN: u8 = 16;

// DHCP server configuration
pub const DHCP_POOL_START: Ipv4Addr = Ipv4Addr::new(10, 42, 0, 10);
pub const DHCP_POOL_END: Ipv4Addr = Ipv4Addr::new(10, 42, 255, 250);
pub const DHCP_LEASE_TIME: u32 = 3600; // 1 hour in seconds

// Network utilities
pub fn subnet_mask() -> Ipv4Addr {
    Ipv4Addr::new(255, 255, 0, 0)
}
