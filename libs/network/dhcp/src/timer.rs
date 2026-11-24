//! Renewal, rebind and expiry deadlines derived from a lease.

use std::time::Instant;

use crate::Lease;

// Absolute times at which a held lease must be renewed, rebound and expired.
#[derive(Debug, Clone, Copy)]
pub(super) struct Deadlines {
    renew: Option<Instant>,
    rebind: Option<Instant>,
    expire: Option<Instant>,
}

impl Deadlines {
    // Creates a deadline set with nothing armed.
    pub(super) const fn new() -> Self {
        Self {
            renew: None,
            rebind: None,
            expire: None,
        }
    }

    // Arms every deadline from the lease lifetimes.
    pub(super) fn arm(&mut self, now: Instant, lease: &Lease) {
        self.renew = now.checked_add(lease.renewal_time);
        self.rebind = now.checked_add(lease.rebind_time);
        self.expire = now.checked_add(lease.lease_time);
    }

    // Clears every deadline.
    pub(super) const fn disarm(&mut self) {
        self.renew = None;
        self.rebind = None;
        self.expire = None;
    }

    // Returns true when the lease has expired.
    pub(super) fn expired(&self, now: Instant) -> bool {
        self.expire.is_some_and(|deadline| now >= deadline)
    }

    // Returns true when the lease must be rebound.
    pub(super) fn rebind_due(&self, now: Instant) -> bool {
        self.rebind.is_some_and(|deadline| now >= deadline)
    }

    // Returns true when the lease must be renewed.
    pub(super) fn renew_due(&self, now: Instant) -> bool {
        self.renew.is_some_and(|deadline| now >= deadline)
    }

    // Returns the earliest armed deadline.
    pub(super) fn next(&self) -> Option<Instant> {
        self.renew.or(self.rebind).or(self.expire)
    }

    // Returns what the earliest deadline would be when armed at `now`.
    pub(super) fn preview(now: Instant, lease: &Lease) -> Option<Instant> {
        let mut deadlines = Self::new();
        deadlines.arm(now, lease);

        deadlines.next()
    }

    // Skips the renewal deadline so the next step waits for rebinding.
    pub(super) const fn skip_renew(&mut self) {
        self.renew = self.rebind;
    }

    // Skips renewal and rebinding so the next step waits for expiry.
    pub(super) const fn wait_expiry(&mut self) {
        self.renew = self.expire;
        self.rebind = self.expire;
    }
}

#[cfg(test)]
mod tests {
    use core::net::Ipv4Addr;
    use core::time::Duration;
    use std::time::{Instant, SystemTime};

    use super::Deadlines;
    use crate::Lease;

    const SERVER: Ipv4Addr = Ipv4Addr::new(10, 0, 0, 1);
    const ASSIGNED: Ipv4Addr = Ipv4Addr::new(10, 0, 0, 2);

    fn lease(renewal: u64, rebind: u64, lease_time: u64) -> Lease {
        Lease {
            obtained_at: SystemTime::UNIX_EPOCH,
            lease_time: Duration::from_secs(lease_time),
            renewal_time: Duration::from_secs(renewal),
            rebind_time: Duration::from_secs(rebind),
            server_ip: SERVER,
            assigned_ip: ASSIGNED,
            prefix_len: 24,
            gateway: None,
            dns_servers: Vec::new(),
        }
    }

    fn at(now: Instant, secs: u64) -> Instant {
        now.checked_add(Duration::from_secs(secs))
            .expect("representable deadline")
    }

    #[test]
    fn armed_deadlines_are_ordered_renew_rebind_expire() {
        // ARRANGE
        let now = Instant::now();
        let mut deadlines = Deadlines::new();

        // ACT
        deadlines.arm(now, &lease(50, 87, 100));

        // ASSERT
        assert_eq!(deadlines.next(), Some(at(now, 50)));
        assert!(deadlines.renew_due(at(now, 50)));
        assert!(!deadlines.rebind_due(at(now, 50)));
        assert!(!deadlines.expired(at(now, 50)));
        assert!(deadlines.rebind_due(at(now, 87)));
        assert!(!deadlines.expired(at(now, 87)));
        assert!(deadlines.expired(at(now, 100)));
    }

    #[test]
    fn skip_renew_waits_for_rebind() {
        // ARRANGE
        let now = Instant::now();
        let mut deadlines = Deadlines::new();
        deadlines.arm(now, &lease(50, 87, 100));

        // ACT
        deadlines.skip_renew();

        // ASSERT
        assert_eq!(deadlines.next(), Some(at(now, 87)));
        assert!(!deadlines.renew_due(at(now, 50)));
        assert!(deadlines.rebind_due(at(now, 87)));
    }

    #[test]
    fn wait_expiry_waits_for_expiry() {
        // ARRANGE
        let now = Instant::now();
        let mut deadlines = Deadlines::new();
        deadlines.arm(now, &lease(50, 87, 100));

        // ACT
        deadlines.wait_expiry();

        // ASSERT
        assert_eq!(deadlines.next(), Some(at(now, 100)));
        assert!(!deadlines.renew_due(at(now, 50)));
        assert!(!deadlines.rebind_due(at(now, 87)));
        assert!(deadlines.expired(at(now, 100)));
    }

    #[test]
    fn overflowing_lifetimes_leave_no_deadline() {
        // ARRANGE
        let now = Instant::now();
        let mut deadlines = Deadlines::new();

        // ACT
        deadlines.arm(now, &lease(u64::MAX, u64::MAX, u64::MAX));

        // ASSERT
        assert_eq!(deadlines.next(), None);
        assert!(!deadlines.renew_due(now));
        assert!(!deadlines.rebind_due(now));
        assert!(!deadlines.expired(now));
    }

    #[test]
    fn disarm_clears_every_deadline() {
        // ARRANGE
        let now = Instant::now();
        let mut deadlines = Deadlines::new();
        deadlines.arm(now, &lease(50, 87, 100));

        // ACT
        deadlines.disarm();

        // ASSERT
        assert_eq!(deadlines.next(), None);
    }
}
