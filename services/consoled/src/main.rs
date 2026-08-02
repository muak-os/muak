//! Console status display daemon.

mod app;
mod input;
mod log;
mod render;
mod state;
mod tty;

use core::time::Duration;

use anyhow::Context as _;
use granola::Health;
use tokio::signal::unix::{SignalKind, signal};
use tokio::sync::mpsc;
use tokio::time::interval;

use crate::input::InputEvent;

const POLL_INTERVAL: Duration = Duration::from_secs(2);

/// Events that drive the daemon's main loop.
enum DaemonEvent {
    Tick,
    Kmsg(String),
    Input(InputEvent),
    Shutdown,
}

#[granola::service("consoled")]
#[tokio::main]
async fn main(notifier: NotifyClient) -> Result<()> {
    notifier.status("Initializing", Health::Healthy)?;

    let Some(tty) = tty::Tty::open().context("Failed to open TTY")? else {
        kmsg::info!("No VGA console available, exiting.");
        return Ok(());
    };
    let mut app = app::App::new(tty).context("Failed to initialize app")?;

    notifier.ready()?;

    let mut input_rx =
        input::spawn(app.tty().file_arc()).context("Failed to spawn input reader")?;
    let mut kmsg_rx = app::spawn_kmsg_reader()?;

    let (event_tx, mut event_rx) = mpsc::unbounded_channel();

    // Periodic system-state refresh.
    let tick_tx = event_tx.clone();
    tokio::spawn(async move {
        let mut interval = interval(POLL_INTERVAL);
        interval.tick().await;
        loop {
            interval.tick().await;
            if tick_tx.send(DaemonEvent::Tick).is_err() {
                break;
            }
        }
    });

    // Forward kernel log lines into the event stream.
    let kmsg_tx = event_tx.clone();
    tokio::spawn(async move {
        while let Some(line) = kmsg_rx.recv().await {
            let _send = kmsg_tx.send(DaemonEvent::Kmsg(line));
        }
    });

    // Forward input events into the event stream.
    let input_tx = event_tx.clone();
    tokio::spawn(async move {
        while let Some(event) = input_rx.recv().await {
            let _send = input_tx.send(DaemonEvent::Input(event));
        }
    });

    // Request a clean shutdown on SIGTERM.
    let term_tx = event_tx.clone();
    tokio::spawn(async move {
        if let Ok(mut sigterm) = signal(SignalKind::terminate()) {
            sigterm.recv().await;
            let _send = term_tx.send(DaemonEvent::Shutdown);
        }
    });

    // Request a clean shutdown on SIGINT.
    let int_tx = event_tx.clone();
    tokio::spawn(async move {
        if let Ok(mut sigint) = signal(SignalKind::interrupt()) {
            sigint.recv().await;
            let _send = int_tx.send(DaemonEvent::Shutdown);
        }
    });
    drop(event_tx);

    loop {
        match event_rx.recv().await {
            Some(DaemonEvent::Tick) => app.handle_tick(),
            Some(DaemonEvent::Kmsg(line)) => app.handle_kmsg(line),
            Some(DaemonEvent::Input(event)) => app.handle_input(event),
            Some(DaemonEvent::Shutdown) | None => break,
        }
    }

    app.shutdown()?;

    Ok(())
}
