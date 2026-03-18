//! Application state and event handlers for the console display daemon.

use std::io::Write;

use anyhow::{Context, Result};
use crossterm::cursor;
use crossterm::queue;
use crossterm::style::Print;
use tokio::sync::mpsc;

use crate::input::InputEvent;
use crate::log::buffer::LogBuffer;
use crate::log::{reader, view};
use crate::render::{self, FOOTER_ROWS, PANEL_ROWS, ScrollMode};
use crate::state::{self, PollState};
use crate::tty::Tty;

pub struct App {
    tty: Tty,
    logs: LogBuffer,
    poll_state: PollState,
}

impl App {
    pub fn new(mut tty: Tty) -> Result<Self> {
        queue!(tty, cursor::Hide)?;

        let mut poll_state = PollState::default();
        let (cols, rows) = tty.size();
        let sys_state = state::collect(&mut poll_state);
        render::draw(&mut tty, &sys_state, ScrollMode::Live, cols, rows)
            .context("Initial render failed")?;

        Ok(Self {
            tty,
            logs: LogBuffer::new(),
            poll_state,
        })
    }

    pub fn tty(&self) -> &Tty {
        &self.tty
    }

    pub fn handle_tick(&mut self) {
        let (cols, rows) = self.tty.size();
        let sys_state = state::collect(&mut self.poll_state);
        let mode = self.scroll_mode();
        if let Err(e) = render::draw(&mut self.tty, &sys_state, mode, cols, rows) {
            kmsg::warn!("Render failed: {e}");
        }
        self.redraw_logview();
    }

    pub fn handle_kmsg(&mut self, first: String, rx: &mut mpsc::UnboundedReceiver<String>) {
        self.logs.push(first);
        self.logs.drain_channel(rx);
        if self.logs.is_live() {
            self.redraw_logview();
        }
    }

    pub fn handle_input(&mut self, event: InputEvent) {
        let log_rows = self.log_rows();
        match event {
            InputEvent::Up => self.logs.scroll_up(1, log_rows),
            InputEvent::Down => self.logs.scroll_down(1),
            InputEvent::PageUp => self.logs.scroll_up(log_rows.max(1), log_rows),
            InputEvent::PageDown => self.logs.scroll_down(log_rows.max(1)),
            InputEvent::End | InputEvent::Escape => self.logs.snap_to_live(),
        }
        self.redraw_logview();
    }

    pub fn shutdown(&mut self) -> Result<()> {
        queue!(self.tty, cursor::Show, Print("\x1b[r"))?;
        self.tty
            .flush()
            .context("Failed to flush TTY on shutdown")?;
        Ok(())
    }

    pub fn spawn_kmsg_reader(&self) -> Result<mpsc::UnboundedReceiver<String>> {
        reader::spawn().context("Failed to spawn kmsg reader")
    }

    fn scroll_mode(&self) -> ScrollMode {
        if self.logs.is_live() {
            ScrollMode::Live
        } else {
            ScrollMode::Scrollback
        }
    }

    fn log_rows(&self) -> usize {
        let (_cols, rows) = self.tty.size();
        rows.saturating_sub(PANEL_ROWS + FOOTER_ROWS) as usize
    }

    fn redraw_logview(&mut self) {
        let (cols, rows) = self.tty.size();
        let log_rows = self.log_rows();
        let visible = self.logs.visible_window(log_rows);
        if let Err(e) = view::render(&mut self.tty, visible, cols, rows) {
            kmsg::warn!("Logview render failed: {e}");
        }
    }
}
