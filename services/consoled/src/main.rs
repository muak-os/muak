//! consoled - Console status display daemon.

mod render;
mod state;
mod tty;

use std::io::Write;
use std::time::Duration;

use anyhow::{Context, Result};
use crossterm::cursor;
use crossterm::queue;
use crossterm::style::Print;
use notify::{Health, NotifyClient};
use state::PollState;
use tokio::signal::unix::{SignalKind, signal};

const POLL_INTERVAL: Duration = Duration::from_secs(2);

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<()> {
    kmsg::init("consoled")?;
    kmsg::info!("Starting console display daemon");

    let notifier = NotifyClient::new("consoled")?;
    notifier.status("Initializing", Health::Healthy)?;

    let mut tty = tty::Tty::open().context("Failed to open TTY")?;
    queue!(tty, cursor::Hide)?;

    let (cols, rows) = tty.size();
    let mut poll_state = PollState::default();
    let sys_state = state::collect(&mut poll_state);
    render::draw(&mut tty, &sys_state, cols, rows).context("Initial render failed")?;

    notifier.ready()?;

    let mut sigterm = signal(SignalKind::terminate())?;
    let mut sigint = signal(SignalKind::interrupt())?;
    let mut interval = tokio::time::interval(POLL_INTERVAL);

    interval.tick().await;

    loop {
        tokio::select! {
            _ = sigterm.recv() => {
                kmsg::info!("SIGTERM received, shutting down");
                break;
            }
            _ = sigint.recv() => {
                kmsg::info!("SIGINT received, shutting down");
                break;
            }
            _ = interval.tick() => {
                let (cols, rows) = tty.size();
                let sys_state = state::collect(&mut poll_state);
                if let Err(e) = render::draw(&mut tty, &sys_state, cols, rows) {
                    kmsg::warn!("Render failed: {e}");
                }
            }
        }
    }

    // Reset scroll region to full screen and restore cursor.
    queue!(tty, cursor::Show, Print("\x1b[r"))?;
    tty.flush().context("Failed to flush TTY on shutdown")?;
    notifier.stopping("Graceful shutdown")?;

    Ok(())
}
