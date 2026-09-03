//! Shutdown signal handling for granola-managed services.

use tokio::signal::unix::{SignalKind, signal};

/// Returns a future that resolves when SIGTERM or SIGINT is received.
#[expect(
    clippy::integer_division_remainder_used,
    reason = "tokio::select! macro internals use a remainder when shuffling branch order"
)]
pub async fn shutdown() {
    let Ok(mut sigterm) = signal(SignalKind::terminate()) else {
        return;
    };
    let Ok(mut sigint) = signal(SignalKind::interrupt()) else {
        return;
    };

    tokio::select! {
        biased;

        _ = sigterm.recv() => {
            println!("Received SIGTERM, shutting down");
        }
        _ = sigint.recv() => {
            println!("Received SIGINT, shutting down");
        }
    }
}
