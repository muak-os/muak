//! Shutdown signal handling for granola-managed services.

use tokio::signal::unix::{SignalKind, signal};

/// Returns a future that resolves when SIGTERM or SIGINT is received.
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
            kmsg::info!("Received SIGTERM, shutting down");
        }
        _ = sigint.recv() => {
            kmsg::info!("Received SIGINT, shutting down");
        }
    }
}
