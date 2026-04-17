//! Console status display daemon.

mod app;
mod input;
mod log;
mod render;
mod state;
mod tty;

use std::time::Duration;

use anyhow::Context;
use granola::Health;
use tokio::signal::unix::{SignalKind, signal};

const POLL_INTERVAL: Duration = Duration::from_secs(2);

#[granola::service("consoled")]
#[tokio::main]
async fn main(notifier: NotifyClient) -> Result<()> {
    notifier.status("Initializing", Health::Healthy)?;

    let tty = match tty::Tty::open().context("Failed to open TTY")? {
        Some(tty) => tty,
        None => {
            kmsg::info!("No VGA console available, exiting.");
            return Ok(());
        }
    };
    let mut app = app::App::new(tty).context("Failed to initialize app")?;

    notifier.ready()?;

    let mut input_rx =
        input::spawn(app.tty().file_arc()).context("Failed to spawn input reader")?;
    let mut kmsg_rx = app.spawn_kmsg_reader()?;

    let mut sigterm = signal(SignalKind::terminate())?;
    let mut sigint = signal(SignalKind::interrupt())?;
    let mut interval = tokio::time::interval(POLL_INTERVAL);
    interval.tick().await;

    loop {
        tokio::select! {
            _ = sigterm.recv() => break,
            _ = sigint.recv() => break,
            _ = interval.tick() => {
                app.handle_tick();
            }
            Some(line) = kmsg_rx.recv() => {
                app.handle_kmsg(line, &mut kmsg_rx);
            }
            Some(event) = input_rx.recv() => {
                app.handle_input(event);
            }
        }
    }

    app.shutdown()?;

    Ok(())
}
