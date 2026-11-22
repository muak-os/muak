//! Retry policy for the initial `DHCPv4` exchange.

use core::time::Duration;

use crate::Failure as DhcpFailure;
use crate::lease::Lease;

pub(super) const MAX_ATTEMPTS: u32 = 3;

const RETRY_BASE: Duration = Duration::from_secs(2);
const RETRY_MAX: Duration = Duration::from_mins(2);

// What the method should do after one acquisition attempt.
pub(super) enum Outcome {
    // The server granted a lease.
    Acquired(Lease),
    // The attempt failed and may be retried after the delay.
    Retry(Duration),
    // The attempt failed for the last time.
    Failed(DhcpFailure),
}

pub(super) fn classify(attempt: u32, result: crate::Result<Lease>) -> Outcome {
    match result {
        Ok(lease) => Outcome::Acquired(lease),
        Err(error) if attempt >= MAX_ATTEMPTS => Outcome::Failed(error),
        Err(_) => Outcome::Retry(backoff(attempt)),
    }
}

fn backoff(attempt: u32) -> Duration {
    let factor = 1_u32
        .checked_shl(attempt.saturating_sub(1))
        .unwrap_or(u32::MAX);

    RETRY_BASE.saturating_mul(factor).min(RETRY_MAX)
}

#[cfg(test)]
mod tests {
    use core::time::Duration;

    use super::{MAX_ATTEMPTS, Outcome, backoff, classify};
    use crate::Failure as DhcpFailure;

    #[test]
    fn backoff_doubles_from_the_base_and_saturates() {
        // ARRANGE
        let first = backoff(1);
        let second = backoff(2);

        // ACT
        let saturated = backoff(MAX_ATTEMPTS.saturating_add(1));

        // ASSERT
        assert_eq!(first, Duration::from_secs(2));
        assert_eq!(second, Duration::from_secs(4));
        assert!(saturated <= Duration::from_mins(2));
    }

    #[test]
    fn last_attempt_fails_instead_of_retrying() {
        // ARRANGE
        let result = Err(DhcpFailure::Timeout);

        // ACT
        let outcome = classify(MAX_ATTEMPTS, result);

        // ASSERT
        assert!(matches!(outcome, Outcome::Failed(DhcpFailure::Timeout)));
    }

    #[test]
    fn earlier_attempt_retries() {
        // ARRANGE
        let result = Err(DhcpFailure::Nak);

        // ACT
        let outcome = classify(1, result);

        // ASSERT
        assert!(matches!(outcome, Outcome::Retry(_)));
    }
}
