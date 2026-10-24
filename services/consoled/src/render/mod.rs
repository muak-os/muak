//! Terminal rendering.

mod body;
mod footer;
mod header;

use std::io;

use tui::screen::Screen;
use tui::text::Line;

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
pub fn draw<S: Screen>(
    screen: &mut S,
    state: &SystemState,
    scroll_mode: ScrollMode,
    cols: u16,
    rows: u16,
    separator: &str,
) -> io::Result<()> {
    let scroll_top = PANEL_ROWS.saturating_add(1);
    let scroll_bot = rows.saturating_sub(FOOTER_ROWS);
    let cursor_park = rows.saturating_sub(FOOTER_ROWS).saturating_sub(1);

    screen.set_scroll_region(scroll_top, scroll_bot)?;

    let mut row = 0_u16;
    row = header::draw(screen, state, row)?;
    row = draw_separator(screen, row, separator)?;
    body::draw(screen, state, cols, row)?;
    draw_separator(screen, row.saturating_add(PANEL_BODY_ROWS), separator)?;
    draw_separator(screen, rows.saturating_sub(FOOTER_ROWS), separator)?;
    footer::draw(screen, scroll_mode, cols, rows)?;

    screen.move_to(0, cursor_park)?;

    screen.flush()
}

fn draw_separator<S: Screen>(screen: &mut S, row: u16, separator: &str) -> io::Result<u16> {
    screen.clear_line(row)?;
    let line = Line::plain(0, separator);
    screen.write_line(row, &line)?;

    Ok(row.saturating_add(1))
}

#[cfg(test)]
mod tests {
    use firmware::secureboot::Status;
    use tui::screen::Ansi;
    use tui::size::Size;

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
            secure_boot: Status::Disabled,
            secure_boot_mismatch: None,
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

    fn screen() -> Ansi<Vec<u8>> {
        Ansi::new(Vec::new(), Size::new(DEFAULT_COLS, DEFAULT_ROWS))
    }

    #[test]
    fn draw_produces_output() {
        // ARRANGE
        let state = test_state();
        let mut screen = screen();

        // ACT
        draw(
            &mut screen,
            &state,
            ScrollMode::Live,
            DEFAULT_COLS,
            DEFAULT_ROWS,
            &separator(),
        )
        .unwrap();

        // ASSERT
        assert_ne!(screen.into_inner(), b"");
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
            secure_boot: Status::Disabled,
            secure_boot_mismatch: None,
            ntp_server: None,
            interfaces: Vec::new(),
            gateway: None,
            dns_servers: Vec::new(),
        };
        let mut screen = screen();

        // ACT
        draw(
            &mut screen,
            &state,
            ScrollMode::Live,
            DEFAULT_COLS,
            DEFAULT_ROWS,
            &separator(),
        )
        .unwrap();

        // ASSERT
        assert_ne!(screen.into_inner(), b"");
    }

    #[test]
    fn draw_scrollback_mode_shows_indicator() {
        // ARRANGE
        let state = test_state();
        let mut screen = screen();

        // ACT
        draw(
            &mut screen,
            &state,
            ScrollMode::Scrollback,
            DEFAULT_COLS,
            DEFAULT_ROWS,
            &separator(),
        )
        .unwrap();

        // ASSERT
        let bytes = screen.into_inner();
        let output = String::from_utf8_lossy(&bytes);
        assert!(output.contains("[SCROLLBACK]"));
    }

    #[test]
    fn draw_flushes_the_writer() {
        // ARRANGE
        let state = test_state();
        let mut screen = Ansi::new(
            io::LineWriter::new(Vec::new()),
            Size::new(DEFAULT_COLS, DEFAULT_ROWS),
        );

        // ACT
        draw(
            &mut screen,
            &state,
            ScrollMode::Live,
            DEFAULT_COLS,
            DEFAULT_ROWS,
            &separator(),
        )
        .unwrap();

        // ASSERT
        let writer = screen.into_inner();
        assert!(
            !writer.get_ref().is_empty(),
            "flush must release queued output"
        );
    }
}
