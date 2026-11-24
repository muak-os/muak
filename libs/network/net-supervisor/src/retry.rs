//! Exponential backoff for discovery retries.

use std::time::{Duration, Instant};

use crate::Supervisor;

const BASE: Duration = Duration::from_secs(1);
const MAX: Duration = Duration::from_secs(10);

/// Records one failed attempt and schedules the next discovery.
pub(crate) fn schedule(supervisor: &mut Supervisor, now: Instant) {
    let attempt = supervisor.retry_attempts.saturating_add(1);
    supervisor.retry_attempts = attempt;
    supervisor.retry_at = now.checked_add(delay(attempt));
}

/// Clears the pending retry deadline after discovery produced links.
pub(crate) const fn succeeded(supervisor: &mut Supervisor) {
    supervisor.retry_at = None;
}

/// Clears the deadline and the attempt counter after a full recovery.
pub(crate) const fn clear(supervisor: &mut Supervisor) {
    supervisor.retry_at = None;
    supervisor.retry_attempts = 0;
}

fn delay(number: u32) -> Duration {
    let multiplier = 1_u32 << number.saturating_sub(1).min(5);

    BASE.checked_mul(multiplier).unwrap_or(MAX).min(MAX)
}
