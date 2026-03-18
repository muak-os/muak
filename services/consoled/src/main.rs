//! consoled - Console status display daemon.

mod input;
mod kmsg_reader;
mod logview;
mod render;
mod state;
mod tty;

use std::collections::VecDeque;
use std::io::Write;
use std::time::Duration;

use anyhow::{Context, Result};
use crossterm::cursor;
use crossterm::queue;
use crossterm::style::Print;
use input::InputEvent;
use notify::{Health, NotifyClient};
use render::ScrollMode;
use state::PollState;
use tokio::signal::unix::{SignalKind, signal};

const POLL_INTERVAL: Duration = Duration::from_secs(2);
const RING_CAP: usize = 10_000;

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
    render::draw(&mut tty, &sys_state, ScrollMode::Live, cols, rows)
        .context("Initial render failed")?;

    notifier.ready()?;

    let mut input_rx = input::spawn(tty.file_arc()).context("Failed to spawn input reader")?;
    let mut kmsg_rx = kmsg_reader::spawn().context("Failed to spawn kmsg reader")?;

    let mut sigterm = signal(SignalKind::terminate())?;
    let mut sigint = signal(SignalKind::interrupt())?;
    let mut interval = tokio::time::interval(POLL_INTERVAL);

    interval.tick().await;

    let mut ring: VecDeque<String> = VecDeque::with_capacity(RING_CAP);
    let mut scroll_offset: usize = 0;

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
                if let Err(e) = render::draw(&mut tty, &sys_state, scroll_mode(scroll_offset), cols, rows) {
                    kmsg::warn!("Render failed: {e}");
                }
                redraw_logview(&mut tty, &ring, scroll_offset, cols, rows);
            }
            Some(line) = kmsg_rx.recv() => {
                if ring.len() == RING_CAP {
                    ring.pop_front();
                }
                ring.push_back(line);

                // Drain any remaining buffered lines before rendering once.
                while let Ok(extra) = kmsg_rx.try_recv() {
                    if ring.len() == RING_CAP {
                        ring.pop_front();
                    }
                    ring.push_back(extra);
                }

                if scroll_offset == 0 {
                    let (cols, rows) = tty.size();
                    redraw_logview(&mut tty, &ring, scroll_offset, cols, rows);
                }
            }
            Some(event) = input_rx.recv() => {
                let (cols, rows) = tty.size();
                let log_rows = log_rows(rows);

                match event {
                    InputEvent::Up => {
                        scroll_offset =
                            (scroll_offset + 1).min(ring.len().saturating_sub(log_rows));
                    }
                    InputEvent::Down => {
                        scroll_offset = scroll_offset.saturating_sub(1);
                    }
                    InputEvent::PageUp => {
                        let page = log_rows.max(1);
                        scroll_offset =
                            (scroll_offset + page).min(ring.len().saturating_sub(log_rows));
                    }
                    InputEvent::PageDown => {
                        let page = log_rows.max(1);
                        scroll_offset = scroll_offset.saturating_sub(page);
                    }
                    InputEvent::End | InputEvent::Escape => {
                        scroll_offset = 0;
                    }
                }

                redraw_logview(&mut tty, &ring, scroll_offset, cols, rows);
            }
        }
    }

    queue!(tty, cursor::Show, Print("\x1b[r"))?;
    tty.flush().context("Failed to flush TTY on shutdown")?;
    notifier.stopping("Graceful shutdown")?;

    Ok(())
}

fn scroll_mode(scroll_offset: usize) -> ScrollMode {
    if scroll_offset == 0 {
        ScrollMode::Live
    } else {
        ScrollMode::Scrollback
    }
}

fn log_rows(rows: u16) -> usize {
    rows.saturating_sub(render::PANEL_ROWS + render::FOOTER_ROWS) as usize
}

fn redraw_logview(
    tty: &mut tty::Tty,
    ring: &VecDeque<String>,
    scroll_offset: usize,
    cols: u16,
    rows: u16,
) {
    if let Err(e) = logview::render_logview(tty, ring, scroll_offset, cols, rows) {
        kmsg::warn!("Logview render failed: {e}");
    }
}
