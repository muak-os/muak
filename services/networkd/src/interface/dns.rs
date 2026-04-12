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
    /// Flushes the current nameserver state to resolv.conf via atomic write.
    pub fn flush(&self) -> Result<()> {
        dns::write_resolv_conf(&self.v4, &self.v6)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dns_state_default_is_empty() {
        // ACT
        let dns = DnsState::default();

        // ASSERT
        assert!(dns.v4.is_empty());
        assert!(dns.v6.is_empty());
    }
}
