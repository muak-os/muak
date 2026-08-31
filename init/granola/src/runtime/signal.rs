//! Shutdown signal handling for granola-managed services.

use tokio::signal::unix::{SignalKind, signal};

/// Returns a future that resolves when SIGTERM or SIGINT is received.
pub async fn shutdown_signal() {
    let Ok(mut sigterm) = signal(SignalKind::terminate()) else {
        return;
    };
    let Ok(mut sigint) = signal(SignalKind::interrupt()) else {
        return;
    };

    tokio::select! {
        _ = sigterm.recv() => {
            println!("Received SIGTERM, shutting down");
        }
        _ = sigint.recv() => {
            println!("Received SIGINT, shutting down");
        }
    }
}
