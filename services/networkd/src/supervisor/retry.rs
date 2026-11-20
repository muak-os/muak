//! Startup retry policy for network initialization.

use core::time::Duration;

use anyhow::Result;
use tokio::time::sleep;

use super::NetworkActorHandle;

/// Initializes the network, retrying with exponential backoff until success.
///
/// # Errors
///
/// Returns an error when the supervisor command channel is closed.
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
    if handle.is_closed() {
        return Some(Err(anyhow::anyhow!("supervisor command channel closed")));
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

#[cfg(test)]
mod tests {
    use tokio::sync::mpsc;
    use tokio::time::timeout;

    use super::*;

    #[test]
    fn delay_doubles_until_the_cap() {
        // ARRANGE
        let base = Duration::from_secs(1);
        let max = Duration::from_secs(10);

        // ACT
        let mut actual = [base; 7];
        for (number, slot) in (1_u32..=7).zip(actual.iter_mut()) {
            *slot = delay(base, max, number);
        }

        // ASSERT
        let expected = [
            Duration::from_secs(1),
            Duration::from_secs(2),
            Duration::from_secs(4),
            Duration::from_secs(8),
            Duration::from_secs(10),
            Duration::from_secs(10),
            Duration::from_secs(10),
        ];
        assert_eq!(actual, expected);
    }

    #[tokio::test]
    async fn initialize_stops_when_the_supervisor_is_gone() {
        // ARRANGE
        let (tx, rx) = mpsc::channel(1);
        drop(rx);
        let handle = NetworkActorHandle { tx };

        // ACT
        let outcome = timeout(
            Duration::from_secs(1),
            initialize(&handle, Duration::from_secs(1), Duration::from_secs(10)),
        )
        .await;

        // ASSERT
        assert!(
            matches!(outcome, Ok(Err(_))),
            "expected a fatal closed-channel error, got {outcome:?}"
        );
    }
}
