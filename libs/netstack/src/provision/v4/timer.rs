//! Renewal, rebind and expiry deadlines derived from a lease.

use dhcp_client::lease::Lease;
use tokio::time::Instant;

// Absolute times at which a held lease must be renewed, rebound and expired.
#[derive(Debug, Clone, Copy, Default)]
pub(super) struct Deadlines {
    renew: Option<Instant>,
    rebind: Option<Instant>,
    expire: Option<Instant>,
}

impl Deadlines {
    pub(super) fn arm(&mut self, lease: &Lease) {
        let now = Instant::now();
        self.renew = now.checked_add(lease.renewal_time);
        self.rebind = now.checked_add(lease.rebind_time);
        self.expire = now.checked_add(lease.lease_time);
    }

    pub(super) const fn disarm(&mut self) {
        self.renew = None;
        self.rebind = None;
        self.expire = None;
    }

    pub(super) fn expired(&self, now: Instant) -> bool {
        self.expire.is_some_and(|deadline| now >= deadline)
    }

    pub(super) fn rebind_due(&self, now: Instant) -> bool {
        self.rebind.is_some_and(|deadline| now >= deadline)
    }

    pub(super) fn renew_due(&self, now: Instant) -> bool {
        self.renew.is_some_and(|deadline| now >= deadline)
    }

    pub(super) fn next(&self) -> Option<Instant> {
        self.renew.or(self.rebind).or(self.expire)
    }

    pub(super) const fn skip_renew(&mut self) {
        self.renew = self.rebind;
    }

    pub(super) const fn wait_expiry(&mut self) {
        self.renew = self.expire;
        self.rebind = self.expire;
    }
}
