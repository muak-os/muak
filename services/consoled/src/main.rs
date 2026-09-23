//! Console status display daemon.

mod app;
mod input;
mod log;
mod render;
mod state;
mod tty;

use core::time::Duration;

use anyhow::Context as _;
use granola::runtime::notify::Health;
use tokio::signal::unix::{SignalKind, signal};
use tokio::sync::mpsc;
use tokio::time::interval;

use crate::app::App;
use crate::input::InputEvent;

const POLL_INTERVAL: Duration = Duration::from_secs(2);

const EVENT_CHANNEL_CAPACITY: usize = 256;

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
    let mut app = App::new(tty).context("Failed to initialize app")?;

    notifier.ready()?;

    let mut input_rx =
        input::spawn(app.tty().file_arc()).context("Failed to spawn input reader")?;
    let mut kmsg_rx = app::spawn_kmsg_reader()?;

    let (event_tx, mut event_rx) = mpsc::channel(EVENT_CHANNEL_CAPACITY);

    // Periodic system-state refresh.
    let tick_tx = event_tx.clone();
    tokio::spawn(async move {
        let mut interval = interval(POLL_INTERVAL);
        interval.tick().await;
        loop {
            interval.tick().await;
            if tick_tx.send(DaemonEvent::Tick).await.is_err() {
                break;
            }
        }
    });

    // Forward kernel log lines into the event stream.
    let kmsg_tx = event_tx.clone();
    tokio::spawn(async move {
        while let Some(line) = kmsg_rx.recv().await {
            if kmsg_tx.send(DaemonEvent::Kmsg(line)).await.is_err() {
                break;
            }
        }
    });

    // Forward input events into the event stream.
    let input_tx = event_tx.clone();
    tokio::spawn(async move {
        while let Some(event) = input_rx.recv().await {
            if input_tx.send(DaemonEvent::Input(event)).await.is_err() {
                break;
            }
        }
    });

    // Request a clean shutdown on SIGTERM.
    let term_tx = event_tx.clone();
    tokio::spawn(async move {
        if let Ok(mut sigterm) = signal(SignalKind::terminate()) {
            sigterm.recv().await;
            let _sent = term_tx.send(DaemonEvent::Shutdown).await;
        }
    });

    // Request a clean shutdown on SIGINT.
    let int_tx = event_tx.clone();
    tokio::spawn(async move {
        if let Ok(mut sigint) = signal(SignalKind::interrupt()) {
            sigint.recv().await;
            let _sent = int_tx.send(DaemonEvent::Shutdown).await;
        }
    });
    drop(event_tx);

    loop {
        let Some(event) = event_rx.recv().await else {
            break;
        };

        let shutdown = apply(&mut app, event) || drain(&mut app, &mut event_rx);
        if shutdown {
            break;
        }
        app.settle();
    }

    app.shutdown()?;

    Ok(())
}

fn apply(app: &mut App, event: DaemonEvent) -> bool {
    match event {
        DaemonEvent::Tick => app.handle_tick(),
        DaemonEvent::Kmsg(line) => app.handle_kmsg(line),
        DaemonEvent::Input(event) => app.handle_input(event),
        DaemonEvent::Shutdown => return true,
    }

    false
}

fn drain(app: &mut App, event_rx: &mut mpsc::Receiver<DaemonEvent>) -> bool {
    while let Ok(event) = event_rx.try_recv() {
        if apply(app, event) {
            return true;
        }
    }

    false
}
