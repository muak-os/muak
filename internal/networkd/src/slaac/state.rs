use std::net::Ipv6Addr;

use tokio::time::Instant;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AddressState {
    Preferred,
    Deprecated,
}

#[derive(Debug, Clone)]
pub struct ManagedAddress {
    pub address: Ipv6Addr,
    pub prefix_len: u8,
    pub state: AddressState,
    pub valid_until: Instant,
    pub preferred_until: Instant,
    pub router: Ipv6Addr,
}

impl ManagedAddress {
    pub fn new(
        address: Ipv6Addr,
        prefix_len: u8,
        router: Ipv6Addr,
        valid_lifetime_secs: u32,
        preferred_lifetime_secs: u32,
    ) -> Self {
        let now = Instant::now();
        Self {
            address,
            prefix_len,
            state: AddressState::Preferred,
            valid_until: now + std::time::Duration::from_secs(valid_lifetime_secs as u64),
            preferred_until: now + std::time::Duration::from_secs(preferred_lifetime_secs as u64),
            router,
        }
    }

    pub fn refresh_lifetimes(&mut self, valid_lifetime_secs: u32, preferred_lifetime_secs: u32) {
        let now = Instant::now();
        let new_valid = now + std::time::Duration::from_secs(valid_lifetime_secs as u64);
        let new_preferred = now + std::time::Duration::from_secs(preferred_lifetime_secs as u64);

        let two_hours = std::time::Duration::from_secs(2 * 60 * 60);
        if valid_lifetime_secs as u64 > 2 * 60 * 60 || new_valid > self.valid_until {
            self.valid_until = new_valid;
        } else if self.valid_until > now + two_hours {
            self.valid_until = now + two_hours;
        }

        self.preferred_until = new_preferred;

        if self.preferred_until > now {
            self.state = AddressState::Preferred;
        }
    }

    pub fn is_valid(&self) -> bool {
        Instant::now() < self.valid_until
    }

    pub fn is_preferred(&self) -> bool {
        self.state == AddressState::Preferred && Instant::now() < self.preferred_until
    }
}

#[derive(Debug, Clone)]
pub struct ManagedRouter {
    pub address: Ipv6Addr,
    pub expires_at: Instant,
}

impl ManagedRouter {
    pub fn new(address: Ipv6Addr, lifetime_secs: u16) -> Self {
        Self {
            address,
            expires_at: Instant::now() + std::time::Duration::from_secs(lifetime_secs as u64),
        }
    }

    pub fn refresh_lifetime(&mut self, lifetime_secs: u16) {
        self.expires_at = Instant::now() + std::time::Duration::from_secs(lifetime_secs as u64);
    }

    pub fn is_valid(&self) -> bool {
        Instant::now() < self.expires_at
    }
}

#[derive(Debug, Clone)]
pub struct ManagedDns {
    pub server: Ipv6Addr,
    pub expires_at: Instant,
}

impl ManagedDns {
    pub fn new(server: Ipv6Addr, lifetime_secs: u32) -> Self {
        Self {
            server,
            expires_at: Instant::now() + std::time::Duration::from_secs(lifetime_secs as u64),
        }
    }

    pub fn refresh_lifetime(&mut self, lifetime_secs: u32) {
        self.expires_at = Instant::now() + std::time::Duration::from_secs(lifetime_secs as u64);
    }

    pub fn is_valid(&self) -> bool {
        Instant::now() < self.expires_at
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_managed_address_creation() {
        let addr = ManagedAddress::new(
            "2001:db8::1".parse().unwrap(),
            64,
            "fe80::1".parse().unwrap(),
            3600,
            1800,
        );
        assert_eq!(addr.state, AddressState::Preferred);
        assert!(addr.is_valid());
        assert!(addr.is_preferred());
    }

    #[test]
    fn test_managed_router_creation() {
        let router = ManagedRouter::new("fe80::1".parse().unwrap(), 1800);
        assert!(router.is_valid());
    }

    #[test]
    fn test_managed_dns_creation() {
        let dns = ManagedDns::new("2620:fe::fe".parse().unwrap(), 3600);
        assert!(dns.is_valid());
    }
}
