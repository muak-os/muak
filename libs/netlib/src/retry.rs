//! Async helpers for retrying fallible operations and polling conditions with backoff.

use std::future::Future;

use thiserror::Error;

#[derive(Debug, Error)]
pub enum Error {
    #[error("{0}")]
    Timeout(String),
    #[error("{0}: {1}")]
    TimeoutWithCause(String, String),
}

pub type Result<T> = std::result::Result<T, Error>;

/// Polls an async check function until it returns `Some`, or fails after `max_retries`.
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
        tokio::time::sleep(std::time::Duration::from_millis(delay_ms)).await;
    }

    Err(Error::Timeout(timeout_msg.to_string()))
}

/// Retries a fallible async operation up to `max_retries` times with a delay between attempts.
pub async fn run<F, Fut, T, E>(
    operation: F,
    max_retries: u8,
    delay_ms: u64,
    timeout_msg: &str,
) -> Result<T>
where
    F: Fn() -> Fut,
    Fut: Future<Output = std::result::Result<T, E>>,
    E: std::fmt::Display,
{
    let mut last_error = None;

    for attempt in 0..max_retries {
        match operation().await {
            Ok(result) => return Ok(result),
            Err(e) => last_error = Some(e),
        }
        if attempt < max_retries - 1 {
            tokio::time::sleep(std::time::Duration::from_millis(delay_ms)).await;
        }
    }

    if let Some(err) = last_error {
        Err(Error::TimeoutWithCause(
            timeout_msg.to_string(),
            err.to_string(),
        ))
    } else {
        Err(Error::Timeout(timeout_msg.to_string()))
    }
}
