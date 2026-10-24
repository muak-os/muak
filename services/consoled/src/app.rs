//! Application state and event handlers for the console display daemon.

use anyhow::{Context as _, Result};
use tokio::sync::mpsc;
use tui::input::Key;
use tui::screen::{Ansi, Screen as _};
use tui::size::Size;

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
        let size = tty.size();
        let mut poll_state = PollState::default();
        let sys_state = state::collect(&mut poll_state);
        let separator = SEPARATOR_CHAR.repeat(usize::from(size.cols));
        let mut screen = Ansi::new(&mut tty, size);
        screen.hide_cursor()?;
        screen.clear_all()?;
        screen.move_to(0, 0)?;
        render::draw(
            &mut screen,
            &sys_state,
            ScrollMode::Live,
            size.cols,
            size.rows,
            &separator,
        )
        .context("Initial render failed")?;

        Ok(Self {
            tty,
            logs: Buffer::new(),
            poll_state,
            separator_cols: size.cols,
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

    /// Applies a key event; rendering is deferred to the next settle.
    pub fn handle_input(&mut self, key: Key) {
        let log_rows = self.log_rows();
        match key {
            Key::Up | Key::Char('k') => self.logs.scroll_up(1, log_rows),
            Key::Down | Key::Char('j') => self.logs.scroll_down(1),
            Key::PageUp => self.logs.scroll_up(log_rows.max(1), log_rows),
            Key::PageDown => self.logs.scroll_down(log_rows.max(1)),
            Key::End | Key::Esc => self.logs.snap_to_live(),
            Key::Left
            | Key::Right
            | Key::Home
            | Key::Enter
            | Key::Tab
            | Key::Backspace
            | Key::Delete
            | Key::Insert
            | Key::Char(_) => {}
        }
        self.log_dirty = true;
    }

    /// Flushes all dirty regions to the terminal once per drained event batch.
    pub fn settle(&mut self) {
        let size = self.tty.size();

        if self.full_dirty {
            self.redraw_panel(size);
            self.full_dirty = false;
            self.log_dirty = true;
        }

        if self.log_dirty {
            self.redraw_logview(size);
            self.log_dirty = false;
        }
    }

    pub fn shutdown(&mut self) -> Result<()> {
        let size = self.tty.size();
        let mut screen = Ansi::new(&mut self.tty, size);
        screen.show_cursor()?;
        screen.reset_scroll_region()?;
        screen.flush().context("Failed to flush TTY on shutdown")?;

        Ok(())
    }

    fn ensure_separator(&mut self, cols: u16) {
        if self.separator_cols != cols {
            self.separator = SEPARATOR_CHAR.repeat(usize::from(cols));
            self.separator_cols = cols;
        }
    }

    fn redraw_panel(&mut self, size: Size) {
        self.ensure_separator(size.cols);
        let sys_state = state::collect(&mut self.poll_state);
        let mode = self.scroll_mode();
        let mut screen = Ansi::new(&mut self.tty, size);
        if let Err(e) = render::draw(
            &mut screen,
            &sys_state,
            mode,
            size.cols,
            size.rows,
            &self.separator,
        ) {
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
        let size = self.tty.size();
        usize::from(size.rows.saturating_sub(PANEL_ROWS + FOOTER_ROWS))
    }

    fn redraw_logview(&mut self, size: Size) {
        let log_rows = self.log_rows();
        let window = self.logs.visible_window(log_rows);
        let mut screen = Ansi::new(&mut self.tty, size);
        if let Err(e) = view::render(&mut screen, window, size.cols, size.rows) {
            kmsg::warn!("Logview render failed: {e}");
        }
    }
}

/// Spawns the kernel log reader and returns its event stream.
pub fn spawn_kmsg_reader() -> Result<mpsc::Receiver<String>> {
    reader::spawn().context("Failed to spawn kmsg reader")
}
