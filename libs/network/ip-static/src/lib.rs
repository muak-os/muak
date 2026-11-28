//! Sans-io, allocation-free static IPv4 and IPv6 provisioning method.

#![warn(missing_docs)]

mod fixed;

use core::net::IpAddr;

use netmodel::MethodState;

/// Maximum number of addresses a configuration holds.
pub const MAX_ADDRESSES: usize = 8;

/// Maximum number of DNS servers a configuration or view holds.
pub const MAX_DNS: usize = 4;

/// Address configuration shared by the static methods.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Config {
    // Populated slots come first; the remaining slots stay `None`.
    addresses: [Option<(IpAddr, u8)>; MAX_ADDRESSES],
    gateway: Option<IpAddr>,
    dns: [Option<IpAddr>; MAX_DNS],
}

impl Config {
    /// Copies a configuration into fixed-capacity storage.
    #[must_use = "the configuration is the only way to build a static method"]
    pub fn new<A: Copy + Into<IpAddr>>(
        addresses: &[(A, u8)],
        gateway: Option<A>,
        dns: &[A],
    ) -> Option<Self> {
        if addresses.len() > MAX_ADDRESSES || dns.len() > MAX_DNS {
            return None;
        }

        let mut address_slots: [Option<(IpAddr, u8)>; MAX_ADDRESSES] = [None; MAX_ADDRESSES];
        for (slot, &(address, prefix_len)) in address_slots.iter_mut().zip(addresses) {
            *slot = Some((address.into(), prefix_len));
        }
        let mut dns_slots: [Option<IpAddr>; MAX_DNS] = [None; MAX_DNS];
        for (slot, &server) in dns_slots.iter_mut().zip(dns) {
            *slot = Some(server.into());
        }

        Some(Self {
            addresses: address_slots,
            gateway: gateway.map(Into::into),
            dns: dns_slots,
        })
    }

    /// Returns the optional default gateway.
    #[must_use = "callers need the configuration to provision routes"]
    pub const fn gateway(&self) -> Option<IpAddr> {
        self.gateway
    }

    /// Iterates the populated address and prefix length pairs in order.
    #[must_use = "callers need the configuration to provision addresses"]
    pub fn addresses(&self) -> impl Iterator<Item = (IpAddr, u8)> + '_ {
        self.addresses.iter().flatten().copied()
    }

    /// Iterates the populated DNS servers in order.
    #[must_use = "callers need the configuration to resolve names"]
    pub fn dns(&self) -> impl Iterator<Item = IpAddr> + '_ {
        self.dns.iter().flatten().copied()
    }

    /// Returns the address and prefix length at `index`, when populated.
    pub(crate) fn address(&self, index: usize) -> Option<(IpAddr, u8)> {
        self.addresses.get(index).copied().flatten()
    }

    /// Returns the number of populated address slots.
    pub(crate) fn address_count(&self) -> usize {
        self.addresses
            .iter()
            .position(Option::is_none)
            .unwrap_or(MAX_ADDRESSES)
    }
}

/// Static configuration a family exposes as its view.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct View {
    address: IpAddr,
    prefix_len: u8,
    gateway: Option<IpAddr>,
    dns: [Option<IpAddr>; MAX_DNS],
}

impl View {
    /// Copies a view for one address into fixed-capacity storage.
    #[must_use = "the view is the only way to report the static configuration"]
    pub fn new<A: Copy + Into<IpAddr>>(
        address: A,
        prefix_len: u8,
        gateway: Option<A>,
        dns: &[A],
    ) -> Option<Self> {
        if dns.len() > MAX_DNS {
            return None;
        }

        let mut dns_slots: [Option<IpAddr>; MAX_DNS] = [None; MAX_DNS];
        for (slot, &server) in dns_slots.iter_mut().zip(dns) {
            *slot = Some(server.into());
        }

        Some(Self {
            address: address.into(),
            prefix_len,
            gateway: gateway.map(Into::into),
            dns: dns_slots,
        })
    }

    /// Returns the primary address.
    #[must_use = "callers report the address"]
    pub const fn address(&self) -> IpAddr {
        self.address
    }

    /// Returns the prefix length in bits.
    #[must_use = "callers report the prefix length"]
    pub const fn prefix_len(&self) -> u8 {
        self.prefix_len
    }

    /// Returns the optional default gateway.
    #[must_use = "callers report the gateway"]
    pub const fn gateway(&self) -> Option<IpAddr> {
        self.gateway
    }

    /// Iterates the populated DNS servers in order.
    #[must_use = "callers report the DNS servers"]
    pub fn dns(&self) -> impl Iterator<Item = IpAddr> + '_ {
        self.dns.iter().flatten().copied()
    }

    /// Copies a configuration into a view for one of its addresses.
    pub(crate) const fn from_config(config: &Config, address: IpAddr, prefix_len: u8) -> Self {
        Self {
            address,
            prefix_len,
            gateway: config.gateway(),
            dns: config.dns,
        }
    }
}

/// Static address provisioning method.
pub struct Static {
    config: Config,
    started: bool,
    pending: Option<Mode>,
    cursor: usize,
    notice: Option<MethodState>,
}

// Event batch the method derives from its configuration for the command it is draining.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Mode {
    Provision,
    Stop,
}

#[cfg(test)]
mod tests {
    use core::net::{IpAddr, Ipv4Addr};

    use crate::{Config, MAX_ADDRESSES, MAX_DNS, View};

    const ADDRESS: Ipv4Addr = Ipv4Addr::new(10, 0, 0, 2);
    const GATEWAY: Ipv4Addr = Ipv4Addr::new(10, 0, 0, 1);
    const DNS: [Ipv4Addr; 2] = [Ipv4Addr::new(10, 0, 0, 53), Ipv4Addr::new(10, 0, 0, 54)];

    #[test]
    fn config_new_rejects_overflow() {
        // ARRANGE
        let addresses = [(ADDRESS, 24_u8); MAX_ADDRESSES + 1];
        let dns = [DNS[0]; MAX_DNS + 1];

        // ACT
        let address_overflow = Config::new(&addresses, Some(GATEWAY), &DNS);
        let dns_overflow = Config::new(&[(ADDRESS, 24)], Some(GATEWAY), &dns);

        // ASSERT
        assert!(address_overflow.is_none());
        assert!(dns_overflow.is_none());
    }

    #[test]
    fn config_new_keeps_input_within_capacity() {
        // ARRANGE
        let addresses = [(ADDRESS, 24_u8)];

        // ACT
        let config = Config::new(&addresses, Some(GATEWAY), &DNS).expect("within capacity");

        // ASSERT
        assert!(config.addresses().eq([(IpAddr::V4(ADDRESS), 24)]));
        assert_eq!(config.gateway(), Some(IpAddr::V4(GATEWAY)));
        assert!(config.dns().eq(DNS.map(IpAddr::V4)));
    }

    #[test]
    fn config_new_accepts_empty_input() {
        // ARRANGE
        let addresses: [(Ipv4Addr, u8); 0] = [];

        // ACT
        let config = Config::new(&addresses, None, &[]).expect("within capacity");

        // ASSERT
        assert!(config.addresses().next().is_none());
        assert_eq!(config.gateway(), None);
        assert!(config.dns().next().is_none());
    }

    #[test]
    fn view_new_rejects_dns_overflow() {
        // ARRANGE
        let dns = [DNS[0]; MAX_DNS + 1];

        // ACT
        let view = View::new(ADDRESS, 24, Some(GATEWAY), &dns);

        // ASSERT
        assert!(view.is_none());
    }

    #[test]
    fn view_is_copy_and_matches_config() {
        // ARRANGE
        let view = View::new(ADDRESS, 24, Some(GATEWAY), &DNS).expect("within capacity");

        // ACT
        let copied = view;
        let reused = view;

        // ASSERT
        assert_eq!(copied, reused);
        assert_eq!(copied.address(), IpAddr::V4(ADDRESS));
        assert_eq!(copied.prefix_len(), 24);
        assert_eq!(copied.gateway(), Some(IpAddr::V4(GATEWAY)));
        assert!(copied.dns().eq(DNS.map(IpAddr::V4)));
        assert!(reused.dns().eq(DNS.map(IpAddr::V4)));
    }

    #[test]
    fn views_from_same_config_compare_equal() {
        // ARRANGE
        let config = Config::new(&[(ADDRESS, 24)], Some(GATEWAY), &DNS).expect("within capacity");

        // ACT
        let first = View::from_config(&config, IpAddr::V4(ADDRESS), 24);
        let second = View::from_config(&config, IpAddr::V4(ADDRESS), 24);

        // ASSERT
        assert_eq!(first, second);
    }

    #[test]
    fn views_with_different_dns_lengths_compare_unequal() {
        // ARRANGE
        let config = Config::new(&[(ADDRESS, 24)], Some(GATEWAY), &DNS).expect("within capacity");
        let full = View::new(ADDRESS, 24, Some(GATEWAY), &DNS).expect("within capacity");
        let short = View::new(ADDRESS, 24, Some(GATEWAY), &DNS[..1]).expect("within capacity");

        // ACT
        let config_view = View::from_config(&config, IpAddr::V4(ADDRESS), 24);

        // ASSERT
        assert!(short.dns().eq(full.dns().take(1)));
        assert_ne!(full, short);
        assert_eq!(full, config_view);
    }
}
