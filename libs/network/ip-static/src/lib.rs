//! Sans-io, allocation-free static IPv4 and IPv6 provisioning method.

#![warn(missing_docs)]

mod fixed;

use core::net::{IpAddr, Ipv4Addr, Ipv6Addr};

use netmodel::MethodState;

/// Maximum number of addresses a configuration holds.
pub const MAX_ADDRESSES: usize = 8;

/// Maximum number of DNS servers a configuration or view holds.
pub const MAX_DNS: usize = 4;

/// Address type that names the value stored in unused fixed-capacity slots.
pub trait Placeholder: Copy {
    /// Value stored in slots that no accessor returns.
    const PLACEHOLDER: Self;
}

impl Placeholder for Ipv4Addr {
    const PLACEHOLDER: Self = Self::UNSPECIFIED;
}

impl Placeholder for Ipv6Addr {
    const PLACEHOLDER: Self = Self::UNSPECIFIED;
}

/// Address configuration shared by the static methods.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Config<A> {
    addresses: [(A, u8); MAX_ADDRESSES],
    address_count: usize,
    gateway: Option<A>,
    dns: [A; MAX_DNS],
    dns_count: usize,
}

impl<A: Copy + Placeholder> Config<A> {
    /// Copies a configuration into fixed-capacity storage.
    #[must_use = "the configuration is the only way to build a static method"]
    pub fn new(addresses: &[(A, u8)], gateway: Option<A>, dns: &[A]) -> Option<Self> {
        if addresses.len() > MAX_ADDRESSES || dns.len() > MAX_DNS {
            return None;
        }

        let mut address_slots = [(A::PLACEHOLDER, 0_u8); MAX_ADDRESSES];
        for (slot, &pair) in address_slots.iter_mut().zip(addresses) {
            *slot = pair;
        }
        let mut dns_slots = [A::PLACEHOLDER; MAX_DNS];
        for (slot, &server) in dns_slots.iter_mut().zip(dns) {
            *slot = server;
        }

        Some(Self {
            addresses: address_slots,
            address_count: addresses.len(),
            gateway,
            dns: dns_slots,
            dns_count: dns.len(),
        })
    }
}

impl<A: Copy> Config<A> {
    /// Returns the configured address and prefix length pairs.
    #[must_use = "callers need the configuration to provision addresses"]
    pub fn addresses(&self) -> &[(A, u8)] {
        self.addresses.get(..self.address_count).unwrap_or_default()
    }

    /// Returns the optional default gateway.
    #[must_use = "callers need the configuration to provision routes"]
    pub const fn gateway(&self) -> Option<A> {
        self.gateway
    }

    /// Returns the configured DNS servers.
    #[must_use = "callers need the configuration to resolve names"]
    pub fn dns(&self) -> &[A] {
        self.dns.get(..self.dns_count).unwrap_or_default()
    }
}

/// Static configuration a family exposes as its view.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct View<A> {
    address: A,
    prefix_len: u8,
    gateway: Option<A>,
    // DNS servers followed by the number of populated slots.
    dns: [A; MAX_DNS],
    dns_count: usize,
}

impl<A: Copy> View<A> {
    /// Copies a view for one address into fixed-capacity storage.
    #[must_use = "the view is the only way to report the static configuration"]
    pub fn new(address: A, prefix_len: u8, gateway: Option<A>, dns: &[A]) -> Option<Self> {
        if dns.len() > MAX_DNS {
            return None;
        }

        let mut dns_slots = [address; MAX_DNS];
        for (slot, &server) in dns_slots.iter_mut().zip(dns) {
            *slot = server;
        }

        Some(Self {
            address,
            prefix_len,
            gateway,
            dns: dns_slots,
            dns_count: dns.len(),
        })
    }

    /// Returns the primary address.
    #[must_use = "callers report the address"]
    pub const fn address(&self) -> A {
        self.address
    }

    /// Returns the prefix length in bits.
    #[must_use = "callers report the prefix length"]
    pub const fn prefix_len(&self) -> u8 {
        self.prefix_len
    }

    /// Returns the optional default gateway.
    #[must_use = "callers report the gateway"]
    pub const fn gateway(&self) -> Option<A> {
        self.gateway
    }

    /// Returns the DNS servers.
    #[must_use = "callers report the DNS servers"]
    pub fn dns(&self) -> &[A] {
        self.dns.get(..self.dns_count).unwrap_or_default()
    }
}

/// Static address method generic over the concrete address type.
pub struct Static<A: Copy + Into<IpAddr>> {
    config: Config<A>,
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
    use core::net::Ipv4Addr;

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
        assert_eq!(config.addresses(), &addresses);
        assert_eq!(config.gateway(), Some(GATEWAY));
        assert_eq!(config.dns(), &DNS);
    }

    #[test]
    fn config_new_accepts_empty_input() {
        // ARRANGE
        let addresses: [(Ipv4Addr, u8); 0] = [];
        let no_dns: &[Ipv4Addr] = &[];

        // ACT
        let config = Config::new(&addresses, None, &[]).expect("within capacity");

        // ASSERT
        assert_eq!(config.addresses(), []);
        assert_eq!(config.gateway(), None);
        assert_eq!(config.dns(), no_dns);
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
        assert_eq!(copied.address(), ADDRESS);
        assert_eq!(copied.prefix_len(), 24);
        assert_eq!(copied.gateway(), Some(GATEWAY));
        assert_eq!(copied.dns(), &DNS);
        assert_eq!(reused.dns(), &DNS);
    }
}
