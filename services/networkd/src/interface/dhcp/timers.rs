//! Lease renewal, rebind and expiry timer handles.

use core::pin::Pin;
use std::time::SystemTime;

use dhcp::lease::Lease;
use tokio::time::{Sleep, sleep};

/// Holds the three timers handles that drive the DHCP renewal state machine.
pub(in crate::interface) struct LeaseTimers {
    pub renew: Option<Pin<Box<Sleep>>>,
    pub rebind: Option<Pin<Box<Sleep>>>,
    pub expire: Option<Pin<Box<Sleep>>>,
}

impl LeaseTimers {
    /// Returns a new instance with all timers disarmed.
    pub(in crate::interface) fn new() -> Self {
        Self {
            renew: None,
            rebind: None,
            expire: None,
        }
    }

    /// Arms all three timers from the deadlines encoded in `lease`.
    pub(in crate::interface) fn arm(&mut self, lease: &Lease) {
        self.disarm();
        let now = SystemTime::now();
        self.renew = deadline_to_sleep(
            now,
            lease
                .obtained_at
                .checked_add(lease.renewal_time)
                .unwrap_or(now),
        );
        self.rebind = deadline_to_sleep(
            now,
            lease
                .obtained_at
                .checked_add(lease.rebind_time)
                .unwrap_or(now),
        );
        self.expire = deadline_to_sleep(now, lease.expiry());
    }

    /// Cancels all active timers.
    pub(in crate::interface) fn disarm(&mut self) {
        self.renew = None;
        self.rebind = None;
        self.expire = None;
    }
}

fn deadline_to_sleep(now: SystemTime, deadline: SystemTime) -> Option<Pin<Box<Sleep>>> {
    let dur = deadline.duration_since(now).ok()?;

    Some(Box::pin(sleep(dur)))
}
