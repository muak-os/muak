//! Terminal rendering.

mod panel;
mod span;

use std::io::{self, Write};

use anyhow::Result;
use crossterm::cursor::MoveTo;
use crossterm::queue;
use crossterm::style::{Print, ResetColor};
use crossterm::terminal::{Clear, ClearType};

use crate::state::SystemState;

/// Total fixed height of the top panel: 1 header + 1 separator + 6 body + 1 separator.
pub const PANEL_ROWS: u16 = 9;

/// Fixed height of the bottom footer: 1 separator + 1 info row.
pub const FOOTER_ROWS: u16 = 2;

const PANEL_BODY_ROWS: u16 = PANEL_ROWS - 3;

#[cfg(test)]
const DEFAULT_COLS: u16 = 80;
#[cfg(test)]
const DEFAULT_ROWS: u16 = 40;

/// Whether the log area is auto-following new entries or pinned at an offset.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScrollMode {
    Live,
    Scrollback,
}

/// Renders the final presentation of the interface.
pub fn draw<W: Write>(
    w: &mut W,
    state: &SystemState,
    scroll_mode: ScrollMode,
    cols: u16,
    rows: u16,
    separator: &str,
) -> Result<()> {
    let scroll_top = PANEL_ROWS.saturating_add(1);
    let scroll_bot = rows.saturating_sub(FOOTER_ROWS);
    let cursor_park = rows.saturating_sub(FOOTER_ROWS).saturating_sub(1);

    queue!(w, Print(format!("\x1b[{scroll_top};{scroll_bot}r")))?;

    let mut row = 0_u16;
    row = panel::draw_header(w, state, row)?;
    row = draw_separator(w, row, separator)?;
    panel::draw_panel_body(w, state, cols, row)?;
    draw_separator(w, row.saturating_add(PANEL_BODY_ROWS), separator)?;
    draw_separator(w, rows.saturating_sub(FOOTER_ROWS), separator)?;
    panel::draw_footer(w, scroll_mode, cols, rows)?;

    queue!(w, ResetColor, MoveTo(0, cursor_park))?;

    w.flush()?;

    Ok(())
}

fn draw_separator(w: &mut impl Write, row: u16, separator: &str) -> io::Result<u16> {
    queue!(
        w,
        MoveTo(0, row),
        Clear(ClearType::CurrentLine),
        ResetColor,
        Print(separator),
    )?;

    Ok(row.saturating_add(1))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::{CpuUsage, MemoryInfo, NetInterface, SystemStatus, Uptime};

    fn test_state() -> SystemState {
        SystemState {
            hostname: "muak-test".to_owned(),
            version: "0.1.0".to_owned(),
            uptime: Uptime {
                days: 1,
                hours: 2,
                minutes: 30,
            },
            cpu: CpuUsage { percent: 14.2 },
            memory: MemoryInfo {
                total_kb: 4_000_000,
                used_kb: 2_500_000,
            },
            system_status: SystemStatus::Maintenance,
            secure_boot: false,
            ntp_server: Some("pool.ntp.org".to_owned()),
            interfaces: vec![NetInterface {
                name: "eth0".to_owned(),
                addresses: vec!["192.168.1.5".to_owned()],
            }],
            gateway: Some("192.168.1.1".to_owned()),
            dns_servers: vec!["1.1.1.1".to_owned()],
        }
    }

    fn separator() -> String {
        "─".repeat(usize::from(DEFAULT_COLS))
    }

    #[test]
    fn draw_produces_output() {
        // ARRANGE
        let state = test_state();
        let mut buf: Vec<u8> = Vec::new();

        // ACT
        draw(
            &mut buf,
            &state,
            ScrollMode::Live,
            DEFAULT_COLS,
            DEFAULT_ROWS,
            &separator(),
        )
        .unwrap();

        // ASSERT
        assert!(!buf.is_empty());
    }

    #[test]
    fn draw_empty_state() {
        // ARRANGE
        let state = SystemState {
            hostname: String::new(),
            version: String::new(),
            uptime: Uptime::default(),
            cpu: CpuUsage::default(),
            memory: MemoryInfo::default(),
            system_status: SystemStatus::Maintenance,
            secure_boot: false,
            ntp_server: None,
            interfaces: Vec::new(),
            gateway: None,
            dns_servers: Vec::new(),
        };
        let mut buf: Vec<u8> = Vec::new();

        // ACT
        draw(
            &mut buf,
            &state,
            ScrollMode::Live,
            DEFAULT_COLS,
            DEFAULT_ROWS,
            &separator(),
        )
        .unwrap();
    }

    #[test]
    fn draw_scrollback_mode_shows_indicator() {
        // ARRANGE
        let state = test_state();
        let mut buf: Vec<u8> = Vec::new();

        // ACT
        draw(
            &mut buf,
            &state,
            ScrollMode::Scrollback,
            DEFAULT_COLS,
            DEFAULT_ROWS,
            &separator(),
        )
        .unwrap();

        // ASSERT
        let output = String::from_utf8_lossy(&buf);
        assert!(output.contains("[SCROLLBACK]"));
    }

    type Delayed = io::LineWriter<Vec<u8>>;

    #[test]
    fn draw_flushes_the_writer() {
        // ARRANGE
        let state = test_state();
        let mut writer: Delayed = io::LineWriter::new(Vec::new());

        // ACT
        draw(
            &mut writer,
            &state,
            ScrollMode::Live,
            DEFAULT_COLS,
            DEFAULT_ROWS,
            &separator(),
        )
        .unwrap();

        // ASSERT
        assert!(
            !writer.get_ref().is_empty(),
            "flush must release queued output"
        );
    }
}
