use anyhow::Result;
use std::future::Future;

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

    anyhow::bail!("{}", timeout_msg)
}

pub async fn retry_operation<F, Fut, T, E>(
    operation: F,
    max_retries: u8,
    delay_ms: u64,
    timeout_msg: &str,
) -> Result<T>
where
    F: Fn() -> Fut,
    Fut: Future<Output = Result<T, E>>,
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
        anyhow::bail!("{}: {}", timeout_msg, err)
    } else {
        anyhow::bail!("{}", timeout_msg)
    }
}
