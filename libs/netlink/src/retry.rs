//! Async helpers for retrying fallible operations and polling conditions with backoff.

use core::fmt::Display;
use core::future::Future;
use core::time::Duration;

use thiserror::Error;
use tokio::time::sleep;

// A second dump on the shared socket is refused with EBUSY.
const EBUSY: i32 = -16;

/// Attempts made for a dump refused because another dump is in progress.
pub const DUMP_ATTEMPTS: u8 = 3;

/// Delay between dump attempts in milliseconds.
pub const DUMP_RETRY_DELAY_MS: u64 = 50;

/// Retry operation failures.
#[derive(Debug, Error)]
pub enum Failure {
    /// Operation timed out.
    #[error("{0}")]
    Timeout(String),
    /// Operation timed out with a cause.
    #[error("{0}: {1}")]
    TimeoutWithCause(String, String),
}

/// Retry operation result type.
pub type Result<T> = core::result::Result<T, Failure>;

/// Returns true when a netlink dump was refused because another dump is in progress.
#[must_use = "the caller decides whether to retry the dump"]
pub fn is_busy(error: &rtnetlink::Error) -> bool {
    matches!(
        error,
        rtnetlink::Error::NetlinkError(inner) if inner.raw_code() == EBUSY
    )
}

/// Polls an async check function until it returns `Some`, or fails after `max_retries`.
///
/// # Errors
///
/// Returns [`Failure::Timeout`] when the condition never becomes available.
pub async fn wait_for_condition<F, Fut, T>(
    check_fn: F,
    max_retries: u8,
    delay_ms: u64,
    timeout_msg: &str,
) -> Result<T>
where
    F: Fn() -> Fut,
    Fut: Future<Output = Option<T>>,
{
    for _ in 0..max_retries {
        if let Some(result) = check_fn().await {
            return Ok(result);
        }
        sleep(Duration::from_millis(delay_ms)).await;
    }

    Err(Failure::Timeout(timeout_msg.to_owned()))
}

/// Retries a fallible async operation up to `max_retries` times with a delay between attempts.
///
/// # Errors
///
/// Returns [`Failure::TimeoutWithCause`] with the last operation error when every attempt fails.
pub async fn run<F, Fut, T, E>(
    operation: F,
    max_retries: u8,
    delay_ms: u64,
    timeout_msg: &str,
) -> Result<T>
where
    F: Fn() -> Fut,
    Fut: Future<Output = core::result::Result<T, E>>,
    E: Display,
{
    let mut last_error = None;
    let last_attempt = max_retries.saturating_sub(1);

    for attempt in 0..max_retries {
        match operation().await {
            Ok(result) => return Ok(result),
            Err(e) => last_error = Some(e),
        }
        if attempt < last_attempt {
            sleep(Duration::from_millis(delay_ms)).await;
        }
    }

    if let Some(err) = last_error {
        Err(Failure::TimeoutWithCause(
            timeout_msg.to_owned(),
            err.to_string(),
        ))
    } else {
        Err(Failure::Timeout(timeout_msg.to_owned()))
    }
}

/// Retries an operation while `is_retriable` accepts its error.
///
/// # Errors
///
/// Returns the last error when an attempt is not retriable or when every attempt fails.
pub async fn while_retriable<F, Fut, T, E, P>(
    operation: F,
    is_retriable: P,
    max_retries: u8,
    delay_ms: u64,
) -> core::result::Result<T, E>
where
    F: Fn() -> Fut,
    Fut: Future<Output = core::result::Result<T, E>>,
    P: Fn(&E) -> bool,
{
    let mut remaining = max_retries.max(1);

    loop {
        match operation().await {
            Ok(result) => return Ok(result),
            Err(error) if remaining > 1 && is_retriable(&error) => {
                remaining = remaining.saturating_sub(1);
            }
            Err(error) => return Err(error),
        }
        sleep(Duration::from_millis(delay_ms)).await;
    }
}

#[cfg(test)]
mod tests {
    use core::future::ready;
    use core::num::NonZeroI32;
    use core::sync::atomic::AtomicU8;
    use core::sync::atomic::Ordering::Relaxed;

    use rtnetlink::packet_core::ErrorMessage;

    use super::*;

    #[test]
    fn is_busy_matches_busy_error() {
        // ARRANGE
        let mut message = ErrorMessage::default();
        message.code = NonZeroI32::new(EBUSY);
        let error = rtnetlink::Error::NetlinkError(message);

        // ACT
        let busy = is_busy(&error);

        // ASSERT
        assert!(busy);
    }

    #[test]
    fn is_busy_rejects_other_errors() {
        // ARRANGE
        let mut message = ErrorMessage::default();
        message.code = NonZeroI32::new(-95_i32);
        let error = rtnetlink::Error::NetlinkError(message);

        // ACT
        let busy = is_busy(&error);

        // ASSERT
        assert!(!busy);
    }

    #[tokio::test]
    async fn wait_for_condition_returns_value_before_timeout() {
        // ARRANGE
        let tries = AtomicU8::new(0);
        let check = || ready((tries.fetch_add(1, Relaxed) >= 2).then_some("ready"));

        // ACT
        let result = wait_for_condition(check, 5, 0, "timed out").await;

        // ASSERT
        assert_eq!(result.expect("condition should succeed"), "ready");
        assert_eq!(tries.load(Relaxed), 3);
    }

    #[tokio::test]
    async fn wait_for_condition_times_out_when_value_never_arrives() {
        // ARRANGE
        let tries = AtomicU8::new(0);
        let check = || ready((tries.fetch_add(1, Relaxed) == u8::MAX).then_some(()));

        // ACT
        let result = wait_for_condition(check, 2, 0, "timed out").await;

        // ASSERT
        assert!(matches!(result, Err(Failure::Timeout(message)) if message == "timed out"));
        assert_eq!(tries.load(Relaxed), 2);
    }

    #[tokio::test]
    async fn run_returns_first_successful_result() {
        // ARRANGE
        let tries = AtomicU8::new(0);
        let next_try = || tries.fetch_add(1, Relaxed);
        let operation = || ready((next_try() > 0).then_some(42_i32).ok_or("not yet"));

        // ACT
        let result = run(operation, 3, 0, "operation timed out").await;

        // ASSERT
        assert_eq!(result.expect("operation should succeed"), 42_i32);
        assert_eq!(tries.load(Relaxed), 2);
    }

    #[tokio::test]
    async fn while_retriable_returns_first_success_without_retrying() {
        // ARRANGE
        let tries = AtomicU8::new(0);
        let next_try = || tries.fetch_add(1, Relaxed);
        let operation = || ready(Ok::<_, u8>(next_try()));

        // ACT
        let result = while_retriable(operation, |_| true, 3, 0).await;

        // ASSERT
        assert_eq!(result, Ok(0));
        assert_eq!(tries.load(Relaxed), 1);
    }

    #[tokio::test]
    async fn while_retriable_succeeds_after_retriable_failures() {
        // ARRANGE
        let tries = AtomicU8::new(0);
        let next_try = || tries.fetch_add(1, Relaxed);
        let operation = || ready(next_try().ge(&2).then_some(7_i32).ok_or("busy"));

        // ACT
        let result = while_retriable(operation, |error| *error == "busy", 3, 0).await;

        // ASSERT
        assert_eq!(result, Ok(7_i32));
        assert_eq!(tries.load(Relaxed), 3);
    }

    #[tokio::test]
    async fn while_retriable_returns_immediately_on_non_retriable_error() {
        // ARRANGE
        let tries = AtomicU8::new(0);
        let next_try = || tries.fetch_add(1, Relaxed);
        let operation = || ready(Err::<(), _>(next_try()));

        // ACT
        let result = while_retriable(operation, |error| *error == u8::MAX, 3, 0).await;

        // ASSERT
        assert_eq!(result, Err(0_u8));
        assert_eq!(tries.load(Relaxed), 1);
    }

    #[tokio::test]
    async fn while_retriable_returns_last_error_after_exhausting_attempts() {
        // ARRANGE
        let tries = AtomicU8::new(0);
        let next_try = || tries.fetch_add(1, Relaxed);
        let operation = || ready(Err::<(), _>(next_try()));

        // ACT
        let result = while_retriable(operation, |_| true, 3, 0).await;

        // ASSERT
        assert_eq!(result, Err(2_u8));
        assert_eq!(tries.load(Relaxed), 3);
    }

    #[tokio::test]
    async fn run_reports_last_error_after_exhausting_retries() {
        // ARRANGE
        let tries = AtomicU8::new(0);
        let next_try = || tries.fetch_add(1, Relaxed);
        let failure = |try_count| format!("failure-{try_count}");
        let operation = || ready(Err::<(), _>(failure(next_try())));

        // ACT
        let result = run(operation, 3, 0, "operation timed out").await;

        // ASSERT
        assert!(matches!(
            result,
            Err(Failure::TimeoutWithCause(message, cause))
                if message == "operation timed out" && cause == "failure-2"
        ));
        assert_eq!(tries.load(Relaxed), 3);
    }
}
