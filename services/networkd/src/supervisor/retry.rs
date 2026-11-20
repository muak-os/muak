//! Startup retry policy for network initialization.

use core::time::Duration;

use anyhow::Result;
use tokio::time::sleep;

use super::NetworkActorHandle;

/// Initializes the network, retrying with exponential backoff until success.
pub(super) async fn initialize(
    handle: &NetworkActorHandle,
    base_delay: Duration,
    max_delay: Duration,
) -> Result<()> {
    let mut number = 1_u32;
    loop {
        if let Some(result) = attempt(handle, number, base_delay, max_delay).await {
            return result;
        }
        number = number.saturating_add(1);
    }
}

async fn attempt(
    handle: &NetworkActorHandle,
    number: u32,
    base_delay: Duration,
    max_delay: Duration,
) -> Option<Result<()>> {
    if handle.initialize().await.is_ok() {
        println!("Network initialized successfully on attempt {number}");
        return Some(Ok(()));
    }
    eprintln!("Network initialization failed (attempt {number})");

    let backoff = delay(base_delay, max_delay, number);
    println!("Retrying in {backoff:?}...");
    sleep(backoff).await;

    None
}

fn delay(base: Duration, max: Duration, number: u32) -> Duration {
    let multiplier = 1_u32 << number.saturating_sub(1).min(5);

    base.checked_mul(multiplier).unwrap_or(max).min(max)
}
