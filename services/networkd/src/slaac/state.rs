//! Lifetime-tracked state types for SLAAC-acquired addresses, routers, and DNS servers.

use core::net::Ipv6Addr;
use core::time::Duration;

use tokio::time::Instant;

/// An `AddressState` tracking whether an IPv6 address is preferred or deprecated.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AddressState {
    Preferred,
    Deprecated,
}

/// A value of type `T` paired with a monotonic expiry deadline.
#[derive(Debug, Clone)]
pub struct Expiring<T> {
    pub value: T,
    pub expires_at: Instant,
}

/// A router learned via Router Advertisement, tracked until its lifetime expires.
pub type ManagedRouter = Expiring<Ipv6Addr>;

/// A DNS server learned via RDNSS, tracked until its lifetime expires.
pub type ManagedDns = Expiring<Ipv6Addr>;

/// An IPv6 address acquired via SLAAC, with separate preferred and valid lifetimes.
#[derive(Debug, Clone)]
pub struct ManagedAddress {
    pub address: Ipv6Addr,
    pub prefix_len: u8,
    pub state: AddressState,
    pub valid_until: Instant,
    pub preferred_until: Instant,
    pub router: Ipv6Addr,
}

impl<T> Expiring<T> {
    /// Creates a new `Expiring<T>` that expires `lifetime_secs` seconds from now.
    pub fn new(value: T, lifetime_secs: u64) -> Self {
        Self {
            value,
            expires_at: Instant::now()
                .checked_add(Duration::from_secs(lifetime_secs))
                .unwrap_or_else(Instant::now),
        }
    }

    /// Resets the expiry to `lifetime_secs` seconds from now.
    pub fn refresh_lifetime(&mut self, lifetime_secs: u64) {
        self.expires_at = Instant::now()
            .checked_add(Duration::from_secs(lifetime_secs))
            .unwrap_or_else(Instant::now);
    }

    /// Returns `true` if the deadline has not yet passed.
    pub fn is_valid(&self) -> bool {
        Instant::now() < self.expires_at
    }
}
impl ManagedAddress {
    /// Creates a new `ManagedAddress` with RFC 4861 preferred and valid lifetime deadlines.
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
            valid_until: now
                .checked_add(Duration::from_secs(u64::from(valid_lifetime_secs)))
                .unwrap_or(now),
            preferred_until: now
                .checked_add(Duration::from_secs(u64::from(preferred_lifetime_secs)))
                .unwrap_or(now),
            router,
        }
    }

    /// Updates lifetimes per RFC 8981: extends only when new value exceeds two-hour clamp.
    pub fn refresh_lifetimes(&mut self, valid_lifetime_secs: u32, preferred_lifetime_secs: u32) {
        let now = Instant::now();
        let new_valid = now
            .checked_add(Duration::from_secs(u64::from(valid_lifetime_secs)))
            .unwrap_or(now);
        let new_preferred = now
            .checked_add(Duration::from_secs(u64::from(preferred_lifetime_secs)))
            .unwrap_or(now);

        let two_hours = Duration::from_hours(2);
        let clamped = now.checked_add(two_hours).unwrap_or(now);
        if u64::from(valid_lifetime_secs) > 7_200 || new_valid > self.valid_until {
            self.valid_until = new_valid;
        } else {
            self.valid_until = self.valid_until.min(clamped);
        }

        self.preferred_until = new_preferred;

        if self.preferred_until > now {
            self.state = AddressState::Preferred;
        }
    }

    /// Returns `true` if the valid lifetime has not expired.
    pub fn is_valid(&self) -> bool {
        Instant::now() < self.valid_until
    }

    /// Returns `true` if the address is still in the preferred state.
    pub fn is_preferred(&self) -> bool {
        self.state == AddressState::Preferred && Instant::now() < self.preferred_until
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_address(valid_secs: u32, preferred_secs: u32) -> ManagedAddress {
        ManagedAddress::new(
            "2001:db8::1".parse().unwrap(),
            64,
            "fe80::1".parse().unwrap(),
            valid_secs,
            preferred_secs,
        )
    }

    #[test]
    fn expiring_new_is_valid() {
        // ACT
        let e: Expiring<u32> = Expiring::new(42, 3600);

        // ASSERT
        assert!(e.is_valid());
        assert_eq!(e.value, 42);
    }

    #[test]
    fn expiring_zero_lifetime_is_expired() {
        // ACT
        let e: Expiring<u32> = Expiring::new(0, 0);

        // ASSERT
        assert!(!e.is_valid());
    }

    #[test]
    fn expiring_refresh_extends_deadline() {
        // ARRANGE
        let mut e: Expiring<u32> = Expiring::new(1, 10);
        let before = e.expires_at;
        std::thread::sleep(Duration::from_millis(5));

        // ACT
        e.refresh_lifetime(3600);

        // ASSERT
        assert!(e.expires_at > before);
    }

    #[test]
    fn managed_address_creation_is_preferred() {
        // ACT
        let addr = make_address(3600, 1800);

        // ASSERT
        assert_eq!(addr.state, AddressState::Preferred);
        assert!(addr.is_valid());
        assert!(addr.is_preferred());
    }

    #[test]
    fn managed_address_stores_fields() {
        // ACT
        let addr = make_address(3600, 1800);

        // ASSERT
        assert_eq!(addr.address, "2001:db8::1".parse::<Ipv6Addr>().unwrap());
        assert_eq!(addr.prefix_len, 64);
        assert_eq!(addr.router, "fe80::1".parse::<Ipv6Addr>().unwrap());
    }

    #[test]
    fn refresh_lifetimes_extends_when_over_two_hours() {
        // ARRANGE
        let mut addr = make_address(3600, 1800);
        let before_valid = addr.valid_until;
        std::thread::sleep(Duration::from_millis(5));

        // ACT
        addr.refresh_lifetimes(86400, 43200);

        // ASSERT
        assert!(addr.valid_until > before_valid);
    }

    #[test]
    fn refresh_lifetimes_extends_when_new_exceeds_remaining() {
        // ARRANGE
        let mut addr = make_address(100, 50);
        let before_valid = addr.valid_until;
        std::thread::sleep(Duration::from_millis(5));

        // ACT
        addr.refresh_lifetimes(200, 100);

        // ASSERT
        assert!(addr.valid_until > before_valid);
    }

    #[test]
    fn refresh_lifetimes_preferred_always_updated() {
        // ARRANGE
        let mut addr = make_address(3600, 100);
        let before_preferred = addr.preferred_until;
        std::thread::sleep(Duration::from_millis(5));

        // ACT
        addr.refresh_lifetimes(3600, 7200);

        // ASSERT
        assert!(addr.preferred_until > before_preferred);
    }

    #[test]
    fn refresh_lifetimes_restores_preferred_state() {
        // ARRANGE
        let mut addr = make_address(3600, 1800);
        addr.state = AddressState::Deprecated;
        addr.preferred_until = Instant::now()
            .checked_sub(Duration::from_secs(1))
            .unwrap_or_else(Instant::now);

        // ACT
        addr.refresh_lifetimes(7200, 3600);

        // ASSERT
        assert_eq!(addr.state, AddressState::Preferred);
    }

    #[test]
    fn managed_router_creation() {
        // ACT
        let router = ManagedRouter::new("fe80::1".parse().unwrap(), 1800);

        // ASSERT
        assert!(router.is_valid());
        assert_eq!(router.value, "fe80::1".parse::<Ipv6Addr>().unwrap());
    }

    #[test]
    fn managed_router_refresh() {
        // ARRANGE
        let mut router = ManagedRouter::new("fe80::1".parse().unwrap(), 10);
        let before = router.expires_at;
        std::thread::sleep(Duration::from_millis(5));

        // ACT
        router.refresh_lifetime(3600);

        // ASSERT
        assert!(router.expires_at > before);
    }

    #[test]
    fn managed_dns_creation() {
        // ACT
        let dns = ManagedDns::new("2620:fe::fe".parse().unwrap(), 3600);

        // ASSERT
        assert!(dns.is_valid());
        assert_eq!(dns.value, "2620:fe::fe".parse::<Ipv6Addr>().unwrap());
    }

    #[test]
    fn managed_dns_refresh() {
        // ARRANGE
        let mut dns = ManagedDns::new("2620:fe::fe".parse().unwrap(), 10);
        let before = dns.expires_at;
        std::thread::sleep(Duration::from_millis(5));

        // ACT
        dns.refresh_lifetime(3600);

        // ASSERT
        assert!(dns.expires_at > before);
    }

    #[test]
    fn address_state_equality() {
        // ACT / ASSERT
        assert_eq!(AddressState::Preferred, AddressState::Preferred);
        assert_eq!(AddressState::Deprecated, AddressState::Deprecated);
        assert_ne!(AddressState::Preferred, AddressState::Deprecated);
    }

    #[test]
    fn refresh_lifetimes_clamps_to_two_hours_when_remaining_exceeds() {
        // ARRANGE
        let mut addr = make_address(86400, 86400);
        let before_valid = addr.valid_until;

        // ACT
        addr.refresh_lifetimes(100, 100);

        // ASSERT
        assert!(addr.valid_until < before_valid);
        let two_hours = Duration::from_hours(2);
        let margin = Duration::from_secs(5);
        let now = Instant::now();
        let upper = now
            .checked_add(two_hours)
            .and_then(|deadline| deadline.checked_add(margin))
            .unwrap_or(now);
        let lower = now
            .checked_add(two_hours)
            .and_then(|deadline| deadline.checked_sub(margin))
            .unwrap_or(now);
        assert!(addr.valid_until <= upper);
        assert!(addr.valid_until >= lower);
    }

    #[test]
    fn refresh_lifetimes_leaves_valid_unchanged_when_within_two_hours() {
        // ARRANGE
        let mut addr = make_address(3600, 3600);
        let before_valid = addr.valid_until;

        // ACT
        addr.refresh_lifetimes(100, 100);

        // ASSERT
        assert_eq!(addr.valid_until, before_valid);
    }
}
