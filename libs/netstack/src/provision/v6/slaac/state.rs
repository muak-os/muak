//! Lifetime-tracked state for SLAAC addresses, routers, and DNS servers.

use core::net::Ipv6Addr;
use core::time::Duration;

use tokio::time::Instant;

/// Tracks whether an address is still preferred or already deprecated.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum AddressState {
    Preferred,
    Deprecated,
}

/// A value paired with a monotonic expiry deadline.
#[derive(Debug, Clone)]
pub(super) struct Expiring<T> {
    pub(super) value: T,
    pub(super) expires_at: Instant,
}

pub(super) type ManagedRouter = Expiring<Ipv6Addr>;
pub(super) type ManagedDns = Expiring<Ipv6Addr>;

/// An address acquired from a router advertisement with its own lifetimes.
#[derive(Debug, Clone)]
pub(super) struct ManagedAddress {
    pub(super) address: Ipv6Addr,
    pub(super) prefix_len: u8,
    pub(super) state: AddressState,
    pub(super) valid_until: Instant,
    pub(super) preferred_until: Instant,
    pub(super) router: Ipv6Addr,
}

impl<T> Expiring<T> {
    pub(super) fn new(value: T, lifetime_secs: u64) -> Self {
        Self {
            value,
            expires_at: deadline(lifetime_secs),
        }
    }

    pub(super) fn refresh_lifetime(&mut self, lifetime_secs: u64) {
        self.expires_at = deadline(lifetime_secs);
    }

    pub(super) fn is_valid(&self) -> bool {
        Instant::now() < self.expires_at
    }
}

impl ManagedAddress {
    pub(super) fn new(
        address: Ipv6Addr,
        prefix_len: u8,
        router: Ipv6Addr,
        valid_lifetime_secs: u32,
        preferred_lifetime_secs: u32,
    ) -> Self {
        Self {
            address,
            prefix_len,
            state: AddressState::Preferred,
            valid_until: deadline(u64::from(valid_lifetime_secs)),
            preferred_until: deadline(u64::from(preferred_lifetime_secs)),
            router,
        }
    }

    /// Updates lifetimes per RFC 8981: extends only when the new value exceeds two hours.
    pub(super) fn refresh_lifetimes(
        &mut self,
        valid_lifetime_secs: u32,
        preferred_lifetime_secs: u32,
    ) {
        let now = Instant::now();
        let new_valid = deadline(u64::from(valid_lifetime_secs));
        let new_preferred = deadline(u64::from(preferred_lifetime_secs));

        let clamped = now
            .checked_add(Duration::from_hours(2))
            .unwrap_or_else(Instant::now);
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

    pub(super) fn is_valid(&self) -> bool {
        Instant::now() < self.valid_until
    }

    pub(super) fn is_preferred(&self) -> bool {
        self.state == AddressState::Preferred && Instant::now() < self.preferred_until
    }
}

fn deadline(lifetime_secs: u64) -> Instant {
    Instant::now()
        .checked_add(Duration::from_secs(lifetime_secs))
        .unwrap_or_else(Instant::now)
}
