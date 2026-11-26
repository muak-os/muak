//! Nameserver addresses captured from the last resolver write.

use alloc::vec::Vec;
use core::net::{Ipv4Addr, Ipv6Addr};

/// Nameserver addresses the supervisor last asked the service to write.
#[derive(Debug, Default)]
pub(crate) struct Nameservers {
    v4: Vec<Ipv4Addr>,
    v6: Vec<Ipv6Addr>,
}

impl Nameservers {
    /// Replaces the stored payload with the one the supervisor produced.
    pub(crate) fn store(&mut self, v4: &[Ipv4Addr], v6: &[Ipv6Addr]) {
        self.v4.clear();
        self.v4.extend_from_slice(v4);
        self.v6.clear();
        self.v6.extend_from_slice(v6);
    }

    /// Returns the stored IPv4 nameservers.
    pub(crate) fn v4(&self) -> &[Ipv4Addr] {
        &self.v4
    }

    /// Returns the stored IPv6 nameservers.
    pub(crate) fn v6(&self) -> &[Ipv6Addr] {
        &self.v6
    }
}
