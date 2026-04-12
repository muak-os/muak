//! Actor-local DNS nameserver state with resolv.conf flush.

use std::net::{Ipv4Addr, Ipv6Addr};

use anyhow::Result;

use crate::dns;

/// Tracks the current set of DNS nameservers for one interface actor.
#[derive(Debug, Clone, Default)]
pub struct DnsState {
    pub v4: Vec<Ipv4Addr>,
    pub v6: Vec<Ipv6Addr>,
}

impl DnsState {
    /// Replaces the IPv4 nameserver list and flushes resolv.conf.
    pub fn update_v4(&mut self, servers: Vec<Ipv4Addr>) -> Result<()> {
        self.v4 = servers;
        self.flush()
    }

    /// Replaces the IPv6 nameserver list and flushes resolv.conf.
    pub fn update_v6(&mut self, servers: Vec<Ipv6Addr>) -> Result<()> {
        self.v6 = servers;
        self.flush()
    }

    /// Flushes the current nameserver state to resolv.conf via atomic write.
    pub fn flush(&self) -> Result<()> {
        dns::write_resolv_conf(&self.v4, &self.v6)
    }
}

#[cfg(test)]
mod tests {
    use std::net::{Ipv4Addr, Ipv6Addr};

    use super::*;

    #[test]
    fn dns_state_default_is_empty() {
        // ACT
        let dns = DnsState::default();

        // ASSERT
        assert!(dns.v4.is_empty());
        assert!(dns.v6.is_empty());
    }

    #[test]
    fn update_v4_replaces_servers() {
        // ARRANGE
        let mut dns = DnsState::default();
        let servers = vec![Ipv4Addr::new(8, 8, 8, 8)];

        // ACT
        let _ = dns.update_v4(servers.clone());

        // ASSERT
        assert_eq!(dns.v4, servers);
    }

    #[test]
    fn update_v6_replaces_servers() {
        // ARRANGE
        let mut dns = DnsState::default();
        let servers = vec![Ipv6Addr::new(0x2001, 0x4860, 0x4860, 0, 0, 0, 0, 0x8888)];

        // ACT
        let _ = dns.update_v6(servers.clone());

        // ASSERT
        assert_eq!(dns.v6, servers);
    }
}
