//! Actor-local DNS nameserver state with resolv.conf flush.

use std::net::{Ipv4Addr, Ipv6Addr};
use std::path::PathBuf;

use anyhow::Result;

use crate::dns::{self, RESOLV_CONF_PATH};

/// Tracks the current set of DNS nameservers for one interface actor.
#[derive(Debug, Clone)]
pub struct DnsState {
    pub v4: Vec<Ipv4Addr>,
    pub v6: Vec<Ipv6Addr>,
    resolv_conf: PathBuf,
}

impl Default for DnsState {
    fn default() -> Self {
        Self {
            v4: Vec::new(),
            v6: Vec::new(),
            resolv_conf: PathBuf::from(RESOLV_CONF_PATH),
        }
    }
}

impl DnsState {
    /// Creates a `DnsState` that writes to `path` instead of the default `/run/resolv.conf`.
    pub fn with_path(path: PathBuf) -> Self {
        Self {
            resolv_conf: path,
            ..Self::default()
        }
    }

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
        dns::write_resolv_conf(&self.resolv_conf, &self.v4, &self.v6)
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
    fn update_v4_replaces_servers_and_writes_file() {
        // ARRANGE
        let tmp = tempfile::tempdir().expect("tempdir");
        let path = tmp.path().join("resolv.conf");
        let mut dns = DnsState::with_path(path.clone());
        let servers = vec![Ipv4Addr::new(8, 8, 8, 8)];

        // ACT
        dns.update_v4(servers.clone()).expect("update failed");

        // ASSERT
        assert_eq!(dns.v4, servers);
        let content = std::fs::read_to_string(&path).expect("read failed");
        assert!(content.contains("nameserver 8.8.8.8"));
    }

    #[test]
    fn update_v6_replaces_servers_and_writes_file() {
        // ARRANGE
        let tmp = tempfile::tempdir().expect("tempdir");
        let path = tmp.path().join("resolv.conf");
        let mut dns = DnsState::with_path(path.clone());
        let servers = vec![Ipv6Addr::new(0x2001, 0x4860, 0x4860, 0, 0, 0, 0, 0x8888)];

        // ACT
        dns.update_v6(servers.clone()).expect("update failed");

        // ASSERT
        assert_eq!(dns.v6, servers);
        let content = std::fs::read_to_string(&path).expect("read failed");
        assert!(content.contains("nameserver 2001:4860:4860::8888"));
    }
}
