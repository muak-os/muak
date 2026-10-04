//! Retry policy and backoff for transient registry failures.

use core::time::Duration;
use std::time::{SystemTime, UNIX_EPOCH};

use hyper::http::StatusCode;

use crate::error::ClientError;

/// Attempts per request by default, counting the first try.
const DEFAULT_ATTEMPTS: u32 = 5;

/// Delay before the first retry.
const BASE_DELAY: Duration = Duration::from_millis(250);

/// Upper bound for a single backoff step, before jitter.
const MAX_DELAY: Duration = Duration::from_secs(4);

/// Pseudo-jitter mask: up to about 33 ms added to every backoff step.
const JITTER_MASK: u32 = 0x1FF_FFFF;

/// Budget and curve for retrying transient registry failures.
#[derive(Clone, Copy, Debug)]
pub struct Policy {
    attempts: u32,
    base: Duration,
    max: Duration,
}

impl Default for Policy {
    fn default() -> Self {
        Self {
            attempts: DEFAULT_ATTEMPTS,
            base: BASE_DELAY,
            max: MAX_DELAY,
        }
    }
}

impl Policy {
    /// Builds a policy with the given attempt budget.
    #[must_use]
    pub fn new(attempts: u32) -> Self {
        Self {
            attempts,
            base: BASE_DELAY,
            max: MAX_DELAY,
        }
    }

    /// Total attempt budget, counting the first try.
    #[must_use]
    pub fn attempts(&self) -> u32 {
        self.attempts
    }

    /// Backoff delay before retry `attempt` (zero-based), doubling with jitter.
    #[must_use]
    pub fn delay(&self, attempt: u32) -> Duration {
        let factor = u128::from(2_u8).checked_pow(attempt).unwrap_or(u128::MAX);
        let scaled = self
            .base
            .as_millis()
            .saturating_mul(factor)
            .min(u128::from(u64::MAX));
        let raw = Duration::from_millis(u64::try_from(scaled).unwrap_or(u64::MAX));

        raw.min(self.max).saturating_add(jitter())
    }
}

/// Whether the failure may succeed on a later attempt.
#[must_use]
pub fn is_retryable(error: &ClientError) -> bool {
    match *error {
        ClientError::Network(_) => true,
        ClientError::Status { status, .. } => is_retryable_status(status),
        ClientError::Auth { .. } | ClientError::Download(_) | ClientError::Push(_) => false,
    }
}

/// Delay to wait before the next attempt, or `None` when the budget is spen or the failure is permanent.
pub(crate) fn next_retry(policy: &Policy, error: &ClientError, attempt: u32) -> Option<Duration> {
    if attempt.saturating_add(1) >= policy.attempts() || !is_retryable(error) {
        return None;
    }

    Some(policy.delay(attempt))
}

fn is_retryable_status(status: u16) -> bool {
    let Ok(code) = StatusCode::from_u16(status) else {
        return false;
    };

    code.is_server_error()
        || code == StatusCode::TOO_MANY_REQUESTS
        || code == StatusCode::REQUEST_TIMEOUT
}

fn jitter() -> Duration {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0_u32, |since| since.subsec_nanos());

    Duration::from_nanos(u64::from(nanos & JITTER_MASK))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn network_failures_and_transient_statuses_are_retryable() {
        // ARRANGE
        let retryable = [
            ClientError::Network("connection refused".to_owned()),
            ClientError::Status {
                status: 500,
                url: "http://registry/v2/".to_owned(),
            },
            ClientError::Status {
                status: 429,
                url: "http://registry/v2/".to_owned(),
            },
            ClientError::Status {
                status: 408,
                url: "http://registry/v2/".to_owned(),
            },
        ];
        let permanent = [
            ClientError::Status {
                status: 404,
                url: "http://registry/v2/".to_owned(),
            },
            ClientError::Auth {
                registry: "registry".to_owned(),
                details: "denied".to_owned(),
            },
            ClientError::Push("rejected".to_owned()),
            ClientError::Download("redirect failed".to_owned()),
        ];

        // ACT / ASSERT
        for error in &retryable {
            assert!(is_retryable(error), "{error:?} must be retryable");
        }
        for error in &permanent {
            assert!(!is_retryable(error), "{error:?} must be permanent");
        }
    }

    #[test]
    fn delay_doubles_until_the_cap_and_never_exceeds_it() {
        // ARRANGE
        let policy = Policy::default();

        // ACT
        let delays = [
            policy.delay(0),
            policy.delay(1),
            policy.delay(2),
            policy.delay(12),
        ];

        // ASSERT
        assert!(delays[1] > delays[0], "backoff must grow");
        assert!(delays[2] > delays[1], "backoff must keep growing");
        assert!(
            delays[3] >= policy.max,
            "backoff must reach the cap for late attempts"
        );
        for delay in delays {
            assert!(
                delay <= policy.max + Duration::from_millis(40),
                "jitter must stay bounded"
            );
        }
    }

    #[test]
    fn next_retry_stops_when_the_budget_is_spent_or_the_error_is_permanent() {
        // ARRANGE
        let policy = Policy::new(2);
        let permanent = ClientError::Status {
            status: 404,
            url: "http://registry/v2/".to_owned(),
        };
        let transient = ClientError::Network("timeout".to_owned());

        // ACT / ASSERT
        assert!(
            next_retry(&policy, &permanent, 0).is_none(),
            "permanent errors must never retry"
        );
        assert!(
            next_retry(&policy, &transient, 0).is_some(),
            "transient errors retry within the budget"
        );
        assert!(
            next_retry(&policy, &transient, 1).is_none(),
            "the budget caps retries"
        );
    }
}
