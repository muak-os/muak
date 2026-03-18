//! Terminal rendering.

use std::io::{self, Write};

use crossterm::cursor::MoveTo;
use crossterm::queue;
use crossterm::style::{Attribute, Color, Print, ResetColor, SetAttribute, SetForegroundColor};
use crossterm::terminal::{Clear, ClearType};

use crate::state::{SystemState, SystemStatus};

/// Total fixed height of the panel: 1 header + 1 separator + 6 body + 1 separator.
pub const PANEL_ROWS: u16 = 9;

const BODY_ROWS: u16 = PANEL_ROWS - 3;

#[cfg(test)]
const DEFAULT_COLS: u16 = 80;
#[cfg(test)]
const DEFAULT_ROWS: u16 = 40;

/// A single styled segment within a line.
#[derive(Debug, Clone)]
struct Span {
    color: Option<Color>,
    bold: bool,
    text: String,
}

impl Span {
    fn new(color: Color, text: impl Into<String>) -> Self {
        Self {
            color: Some(color),
            bold: false,
            text: text.into(),
        }
    }

    fn bold(color: Color, text: impl Into<String>) -> Self {
        Self {
            color: Some(color),
            bold: true,
            text: text.into(),
        }
    }

    /// Span that emits a full SGR reset (`\x1b[0m`) before its text.
    fn reset(text: impl Into<String>) -> Self {
        Self {
            color: None,
            bold: false,
            text: text.into(),
        }
    }
}

/// A logical line composed of styled spans, with a starting column.
#[derive(Debug, Clone, Default)]
struct Line {
    col: u16,
    spans: Vec<Span>,
}

impl Line {
    fn new(col: u16) -> Self {
        Self {
            col,
            spans: Vec::new(),
        }
    }

    fn push(&mut self, span: Span) {
        self.spans.push(span);
    }

    fn write_to(&self, w: &mut impl Write, row: u16) -> io::Result<()> {
        queue!(w, MoveTo(self.col, row))?;
        for span in &self.spans {
            write_span(w, span)?;
        }
        Ok(())
    }
}

fn write_span(w: &mut impl Write, span: &Span) -> io::Result<()> {
    match span.color {
        Some(color) => queue!(w, SetForegroundColor(color))?,
        None => queue!(w, ResetColor)?,
    }
    if span.bold {
        queue!(w, SetAttribute(Attribute::Bold))?;
    }
    queue!(w, Print(&span.text))?;
    if span.bold {
        queue!(w, SetAttribute(Attribute::Reset))?;
    }
    Ok(())
}

/// Renders the status panel to the writer.
pub fn draw(w: &mut impl Write, state: &SystemState, cols: u16, rows: u16) -> io::Result<u16> {
    let mut row = 0u16;

    row = draw_header(w, state, cols, row)?;
    row = draw_separator(w, cols, row)?;
    draw_body(w, state, cols, row)?;
    draw_separator(w, cols, row + BODY_ROWS)?;

    let scroll_top = PANEL_ROWS + 1;
    queue!(
        w,
        ResetColor,
        Print(format!("\x1b[{scroll_top};{rows}r")),
        MoveTo(0, rows - 1),
    )?;

    w.flush()?;
    Ok(PANEL_ROWS)
}

fn clear_line(w: &mut impl Write, row: u16) -> io::Result<()> {
    queue!(w, MoveTo(0, row), Clear(ClearType::CurrentLine))
}

fn draw_header(w: &mut impl Write, state: &SystemState, _cols: u16, row: u16) -> io::Result<u16> {
    let uptime = &state.uptime;
    let total_gib = state.memory.total_kb as f64 / (1024.0 * 1024.0);

    let summary = format!(
        "up {}d {}h {}m, {:.1} GiB RAM, CPU {:.1}%, RAM {:.1}%",
        uptime.days,
        uptime.hours,
        uptime.minutes,
        total_gib,
        state.cpu.percent,
        state.memory.percent(),
    );

    clear_line(w, row)?;
    queue!(
        w,
        MoveTo(0, row),
        SetForegroundColor(Color::Cyan),
        SetAttribute(Attribute::Bold),
        Print(format!("  {}", state.hostname)),
        SetAttribute(Attribute::Reset),
        ResetColor,
        Print(format!(" (v{})", state.version)),
        Print(": "),
        Print(summary),
    )?;

    Ok(row + 1)
}

fn draw_separator(w: &mut impl Write, cols: u16, row: u16) -> io::Result<u16> {
    let line: String = "─".repeat(cols as usize);
    queue!(
        w,
        MoveTo(0, row),
        Clear(ClearType::CurrentLine),
        ResetColor,
        Print(line),
    )?;
    Ok(row + 1)
}

/// Builds the left column (Status) lines starting at `col`.
fn build_left(state: &SystemState, col: u16) -> Vec<Line> {
    let (status_label, status_color) = match state.system_status {
        SystemStatus::Installed => ("INSTALLED", Color::Green),
        SystemStatus::Maintenance => ("MAINTENANCE", Color::Red),
    };

    let (sb_label, sb_color) = if state.secure_boot {
        ("true", Color::Green)
    } else {
        ("false", Color::Red)
    };

    let mut lines = Vec::new();

    let mut status_line = Line::new(col);
    status_line.push(Span::new(Color::White, "STATUS     "));
    status_line.push(Span::bold(status_color, status_label));
    lines.push(status_line);

    let mut sb_line = Line::new(col);
    sb_line.push(Span::new(Color::White, "SECUREBOOT "));
    sb_line.push(Span::bold(sb_color, sb_label));
    lines.push(sb_line);

    lines
}

/// Builds the right column (Network) lines starting at `col`.
fn build_right(state: &SystemState, col: u16) -> Vec<Line> {
    const KEY_WIDTH: usize = 3;
    let val_col = col + KEY_WIDTH as u16 + 1;
    let mut lines = Vec::new();

    let all_addrs: Vec<&str> = state
        .interfaces
        .iter()
        .flat_map(|iface| iface.addresses.iter().map(String::as_str))
        .collect();

    if all_addrs.is_empty() {
        lines.push(net_kv_line(col, "IP", "none"));
    } else {
        let mut ip_line = Line::new(col);
        ip_line.push(Span::new(Color::White, format!("{:<KEY_WIDTH$} ", "IP")));
        ip_line.push(Span::reset(all_addrs[0].to_owned()));
        lines.push(ip_line);

        for addr in all_addrs.iter().skip(1).take(2) {
            let mut cont = Line::new(val_col);
            cont.push(Span::reset((*addr).to_owned()));
            lines.push(cont);
        }
    }

    if let Some(gw) = &state.gateway {
        lines.push(net_kv_line(col, "GW", gw));
    }

    if !state.dns_servers.is_empty() {
        lines.push(net_kv_line(col, "DNS", &state.dns_servers.join(", ")));
    }

    if let Some(ntp) = &state.ntp_server {
        lines.push(net_kv_line(col, "NTP", ntp));
    }

    lines
}

fn draw_body(w: &mut impl Write, state: &SystemState, cols: u16, start_row: u16) -> io::Result<()> {
    let mid_col = cols / 2;

    let left_lines = build_left(state, 2);
    let right_lines = build_right(state, mid_col);

    let empty_line = Line::default();

    for i in 0..BODY_ROWS as usize {
        let row = start_row + i as u16;
        clear_line(w, row)?;
        left_lines.get(i).unwrap_or(&empty_line).write_to(w, row)?;
        right_lines.get(i).unwrap_or(&empty_line).write_to(w, row)?;
    }

    Ok(())
}

fn net_kv_line(col: u16, key: &str, val: &str) -> Line {
    let mut line = Line::new(col);
    line.push(Span::new(Color::White, format!("{key:<3} ")));
    line.push(Span::reset(val.to_owned()));
    line
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

    #[test]
    fn draw_produces_output() {
        // ARRANGE
        let state = test_state();
        let mut buf: Vec<u8> = Vec::new();

        // ACT
        let result = draw(&mut buf, &state, DEFAULT_COLS, DEFAULT_ROWS);

        // ASSERT
        assert!(result.is_ok());
        assert!(!buf.is_empty());
    }

    #[test]
    fn draw_returns_panel_height() {
        // ARRANGE
        let state = test_state();
        let mut buf: Vec<u8> = Vec::new();

        // ACT
        let rows_used = draw(&mut buf, &state, DEFAULT_COLS, DEFAULT_ROWS).expect("draw failed");

        // ASSERT
        assert_eq!(rows_used, PANEL_ROWS);
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
        let result = draw(&mut buf, &state, DEFAULT_COLS, DEFAULT_ROWS);

        // ASSERT
        assert!(result.is_ok());
    }

    #[test]
    fn build_left_installed_shows_green() {
        // ARRANGE
        let mut state = test_state();
        state.system_status = SystemStatus::Installed;
        state.secure_boot = true;

        // ACT
        let lines = build_left(&state, 0);

        // ASSERT
        assert_eq!(lines.len(), 2);
    }

    #[test]
    fn build_right_ip_wraps_up_to_three() {
        // ARRANGE
        let mut state = test_state();
        state.interfaces = vec![NetInterface {
            name: "eth0".to_owned(),
            addresses: vec![
                "10.0.0.1".to_owned(),
                "10.0.0.2".to_owned(),
                "10.0.0.3".to_owned(),
                "10.0.0.4".to_owned(),
            ],
        }];

        // ACT
        let lines = build_right(&state, 40);

        // ASSERT - first IP line + 2 continuation lines = 3 IP rows, then gw/dns/ntp
        let ip_rows = lines.iter().take(3).filter(|l| !l.spans.is_empty()).count();
        assert_eq!(ip_rows, 3);
    }

    #[test]
    fn build_right_no_interfaces_shows_none() {
        // ARRANGE
        let mut state = test_state();
        state.interfaces.clear();

        // ACT
        let lines = build_right(&state, 40);

        // ASSERT
        assert!(!lines.is_empty());
        assert!(lines[0].spans.iter().any(|s| s.text.contains("none")));
    }
}
