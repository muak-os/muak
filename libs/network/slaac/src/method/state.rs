//! Lifetime-tracked state for SLAAC addresses, routers, and DNS servers.

use core::net::Ipv6Addr;
use core::time::Duration;
use std::time::Instant;

/// Tracks whether an address is still preferred or already deprecated.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum AddressState {
    Preferred,
    Deprecated,
}

/// A value paired with a monotonic expiry deadline.
#[derive(Debug, Clone)]
pub(crate) struct Expiring<T> {
    pub(crate) value: T,
    pub(crate) expires_at: Instant,
}

pub(crate) type ManagedRouter = Expiring<Ipv6Addr>;
pub(crate) type ManagedDns = Expiring<Ipv6Addr>;

/// An address acquired from a router advertisement with its own lifetimes.
#[derive(Debug, Clone)]
pub(crate) struct ManagedAddress {
    pub(crate) address: Ipv6Addr,
    pub(crate) prefix_len: u8,
    pub(crate) state: AddressState,
    pub(crate) valid_until: Instant,
    pub(crate) preferred_until: Instant,
    pub(crate) router: Ipv6Addr,
}

impl<T> Expiring<T> {
    pub(crate) fn new(value: T, lifetime_secs: u64, now: Instant) -> Self {
        Self {
            value,
            expires_at: deadline(lifetime_secs, now),
        }
    }

    pub(crate) fn refresh_lifetime(&mut self, lifetime_secs: u64, now: Instant) {
        self.expires_at = deadline(lifetime_secs, now);
    }

    pub(crate) fn is_valid(&self, now: Instant) -> bool {
        now < self.expires_at
    }
}

impl ManagedAddress {
    pub(crate) fn new(
        address: Ipv6Addr,
        prefix_len: u8,
        router: Ipv6Addr,
        valid_lifetime_secs: u32,
        preferred_lifetime_secs: u32,
        now: Instant,
    ) -> Self {
        Self {
            address,
            prefix_len,
            state: AddressState::Preferred,
            valid_until: deadline(u64::from(valid_lifetime_secs), now),
            preferred_until: deadline(u64::from(preferred_lifetime_secs), now),
            router,
        }
    }

    /// Updates lifetimes per RFC 4862 §5.5.3: extends only when the new value exceeds two hours.
    pub(crate) fn refresh_lifetimes(
        &mut self,
        valid_lifetime_secs: u32,
        preferred_lifetime_secs: u32,
        now: Instant,
    ) {
        let new_valid = deadline(u64::from(valid_lifetime_secs), now);
        let new_preferred = deadline(u64::from(preferred_lifetime_secs), now);
        let clamped = deadline(7_200, now);

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

    pub(crate) fn is_valid(&self, now: Instant) -> bool {
        now < self.valid_until
    }

    pub(crate) fn is_preferred(&self, now: Instant) -> bool {
        self.state == AddressState::Preferred && now < self.preferred_until
    }
}

fn deadline(lifetime_secs: u64, now: Instant) -> Instant {
    now.checked_add(Duration::from_secs(lifetime_secs))
        .unwrap_or(now)
}

#[cfg(test)]
mod tests {
    use core::net::Ipv6Addr;
    use core::time::Duration;
    use std::time::Instant;

    use super::{AddressState, Expiring, ManagedAddress};

    const ADDRESS: Ipv6Addr = Ipv6Addr::new(0x2001, 0x0db8, 0, 0, 0, 0, 0, 2);
    const ROUTER: Ipv6Addr = Ipv6Addr::new(0xfe80, 0, 0, 0, 0, 0, 0, 1);

    fn at(now: Instant, secs: u64) -> Instant {
        now.checked_add(Duration::from_secs(secs))
            .expect("representable instant")
    }

    #[test]
    fn expiring_value_lives_until_its_deadline() {
        // ARRANGE
        let now = Instant::now();
        let expiring = Expiring::new(ROUTER, 10, now);

        // ACT
        let before = expiring.is_valid(at(now, 9));
        let after = expiring.is_valid(at(now, 10));

        // ASSERT
        assert_eq!(expiring.expires_at, at(now, 10));
        assert!(before);
        assert!(!after);
    }

    #[test]
    fn refresh_lifetime_moves_the_deadline() {
        // ARRANGE
        let now = Instant::now();
        let mut expiring = Expiring::new(ROUTER, 10, now);

        // ACT
        expiring.refresh_lifetime(30, at(now, 5));

        // ASSERT
        assert_eq!(expiring.expires_at, at(now, 35));
        assert!(expiring.is_valid(at(now, 34)));
    }

    #[test]
    fn address_starts_preferred_and_deprecates() {
        // ARRANGE
        let now = Instant::now();
        let address = ManagedAddress::new(ADDRESS, 64, ROUTER, 100, 10, now);

        // ACT
        let preferred = address.is_preferred(at(now, 9));
        let deprecated = address.is_preferred(at(now, 10));

        // ASSERT
        assert_eq!(address.state, AddressState::Preferred);
        assert!(preferred);
        assert!(!deprecated);
    }

    #[test]
    fn short_refresh_clamps_valid_lifetime_to_two_hours() {
        // ARRANGE
        let now = Instant::now();
        let mut address = ManagedAddress::new(ADDRESS, 64, ROUTER, 100_000, 50_000, now);

        // ACT
        address.refresh_lifetimes(100, 50, at(now, 5));

        // ASSERT
        assert_eq!(address.valid_until, at(now, 7_205));
        assert_eq!(address.preferred_until, at(now, 55));
        assert!(!address.is_preferred(at(now, 55)));
        assert!(address.is_preferred(at(now, 54)));
    }

    #[test]
    fn long_refresh_extends_the_valid_lifetime() {
        // ARRANGE
        let now = Instant::now();
        let mut address = ManagedAddress::new(ADDRESS, 64, ROUTER, 100, 50, now);
        address.refresh_lifetimes(100, 50, at(now, 5));

        // ACT
        address.refresh_lifetimes(9_000, 50, at(now, 6));

        // ASSERT
        assert_eq!(address.valid_until, at(now, 9_006));
    }
}
