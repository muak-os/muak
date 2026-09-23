//! Application state and event handlers for the console display daemon.

use std::io::Write as _;

use anyhow::{Context as _, Result};
use crossterm::cursor;
use crossterm::queue;
use crossterm::style::Print;
use crossterm::terminal::{Clear, ClearType};
use tokio::sync::mpsc;

use crate::input::InputEvent;
use crate::log::buffer::Buffer;
use crate::log::{reader, view};
use crate::render::{self, FOOTER_ROWS, PANEL_ROWS, ScrollMode};
use crate::state::{self, PollState};
use crate::tty::Tty;

const SEPARATOR_CHAR: &str = "─";

pub struct App {
    tty: Tty,
    logs: Buffer,
    poll_state: PollState,
    separator: String,
    separator_cols: u16,
    full_dirty: bool,
    log_dirty: bool,
}

impl App {
    pub fn new(mut tty: Tty) -> Result<Self> {
        queue!(
            tty,
            cursor::Hide,
            Clear(ClearType::All),
            cursor::MoveTo(0, 0)
        )?;

        let mut poll_state = PollState::default();
        let (cols, rows) = tty.size();
        let sys_state = state::collect(&mut poll_state);
        let separator = SEPARATOR_CHAR.repeat(usize::from(cols));
        render::draw(
            &mut tty,
            &sys_state,
            ScrollMode::Live,
            cols,
            rows,
            &separator,
        )
        .context("Initial render failed")?;

        Ok(Self {
            tty,
            logs: Buffer::new(),
            poll_state,
            separator_cols: cols,
            separator,
            full_dirty: false,
            log_dirty: false,
        })
    }

    pub fn tty(&self) -> &Tty {
        &self.tty
    }

    /// Marks the whole panel for redraw at the next settle.
    pub fn handle_tick(&mut self) {
        self.full_dirty = true;
    }

    /// Records a kernel log line.
    pub fn handle_kmsg(&mut self, line: String) {
        self.logs.push(line);
        self.log_dirty = true;
    }

    /// Applies a scroll event; rendering is deferred to the next settle.
    pub fn handle_input(&mut self, event: InputEvent) {
        let log_rows = self.log_rows();
        match event {
            InputEvent::Up => self.logs.scroll_up(1, log_rows),
            InputEvent::Down => self.logs.scroll_down(1),
            InputEvent::PageUp => self.logs.scroll_up(log_rows.max(1), log_rows),
            InputEvent::PageDown => self.logs.scroll_down(log_rows.max(1)),
            InputEvent::End | InputEvent::Escape => self.logs.snap_to_live(),
        }
        self.log_dirty = true;
    }

    /// Flushes all dirty regions to the terminal once per drained event batch.
    pub fn settle(&mut self) {
        let (cols, rows) = self.tty.size();

        if self.full_dirty {
            self.redraw_panel(cols, rows);
            self.full_dirty = false;
            self.log_dirty = true;
        }

        if self.log_dirty {
            self.redraw_logview();
            self.log_dirty = false;
        }
    }

    pub fn shutdown(&mut self) -> Result<()> {
        queue!(self.tty, cursor::Show, Print("\x1b[r"))?;
        self.tty
            .flush()
            .context("Failed to flush TTY on shutdown")?;
        Ok(())
    }

    fn ensure_separator(&mut self, cols: u16) {
        if self.separator_cols != cols {
            self.separator = SEPARATOR_CHAR.repeat(usize::from(cols));
            self.separator_cols = cols;
        }
    }

    fn redraw_panel(&mut self, cols: u16, rows: u16) {
        self.ensure_separator(cols);
        let sys_state = state::collect(&mut self.poll_state);
        let mode = self.scroll_mode();
        if let Err(e) = render::draw(&mut self.tty, &sys_state, mode, cols, rows, &self.separator) {
            kmsg::warn!("Render failed: {e}");
        }
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
        usize::from(rows.saturating_sub(PANEL_ROWS + FOOTER_ROWS))
    }

    fn redraw_logview(&mut self) {
        let (cols, rows) = self.tty.size();
        let log_rows = self.log_rows();
        let window = self.logs.visible_window(log_rows);
        if let Err(e) = view::render(&mut self.tty, window, cols, rows) {
            kmsg::warn!("Logview render failed: {e}");
        }
    }
}

/// Spawns the kernel log reader and returns its event stream.
pub fn spawn_kmsg_reader() -> Result<mpsc::Receiver<String>> {
    reader::spawn().context("Failed to spawn kmsg reader")
}
