//! Shutdown signal handling for granola-managed services.

use tokio::signal::unix::{SignalKind, signal};

/// Returns a future that resolves when SIGTERM or SIGINT is received.
pub async fn shutdown_signal() {
    let mut sigterm = signal(SignalKind::terminate()).expect("failed to register SIGTERM handler");
    let mut sigint = signal(SignalKind::interrupt()).expect("failed to register SIGINT handler");

    tokio::select! {
        _ = sigterm.recv() => {
            println!("Received SIGTERM, shutting down");
        }
        _ = sigint.recv() => {
            println!("Received SIGINT, shutting down");
        }
    }
}
