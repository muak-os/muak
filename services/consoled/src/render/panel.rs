//! Panel drawing of header, status body, and footer rows.

use std::io::{self, Write};

use anyhow::Result;
use crossterm::cursor::MoveTo;
use crossterm::queue;
use crossterm::style::{Attribute, Color, Print, ResetColor, SetAttribute, SetForegroundColor};
use crossterm::terminal::{Clear, ClearType};

use super::span::{Line, Span};
use super::{PANEL_BODY_ROWS, ScrollMode};
use crate::state::{SystemState, SystemStatus};

/// Draws the summary header line and returns the next row.
pub(super) fn draw_header(w: &mut impl Write, state: &SystemState, row: u16) -> Result<u16> {
    let uptime = &state.uptime;
    let total_gib = format_gib(state.memory.total_kb);

    let summary = format!(
        "up {}d {}h {}m, {total_gib} GiB RAM, CPU {:.1}%, RAM {:.1}%",
        uptime.days,
        uptime.hours,
        uptime.minutes,
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

    Ok(row.saturating_add(1))
}

/// Draws the two-column status body between the separators.
pub(super) fn draw_panel_body(
    w: &mut impl Write,
    state: &SystemState,
    cols: u16,
    start_row: u16,
) -> Result<()> {
    let mid_col = cols.div_euclid(2);

    let left_lines = build_left(state, 2);
    let right_lines = build_right(state, mid_col);

    let empty_line = Line::default();

    for i in 0..usize::from(PANEL_BODY_ROWS) {
        let row = start_row.saturating_add(u16::try_from(i).unwrap_or(0));
        clear_line(w, row)?;
        left_lines.get(i).unwrap_or(&empty_line).write_to(w, row)?;
        right_lines.get(i).unwrap_or(&empty_line).write_to(w, row)?;
    }

    Ok(())
}

/// Draws the bottom footer with the scroll-mode indicator.
pub(super) fn draw_footer(
    w: &mut impl Write,
    scroll_mode: ScrollMode,
    cols: u16,
    rows: u16,
) -> Result<()> {
    let info_row = rows.saturating_sub(1);

    let hint = "  \u{2191}/\u{2193} scroll";
    let (mode_label, esc_hint) = match scroll_mode {
        ScrollMode::Live => ("[LIVE]", ""),
        ScrollMode::Scrollback => ("[SCROLLBACK]", "  ESC live"),
    };

    let right = format!("{esc_hint}  {mode_label}  ");
    let hint_len = hint.chars().count();
    let right_len = right.chars().count();
    let padding = usize::from(cols).saturating_sub(hint_len.saturating_add(right_len));

    clear_line(w, info_row)?;
    queue!(
        w,
        MoveTo(0, info_row),
        ResetColor,
        Print(hint),
        Print(" ".repeat(padding)),
        SetForegroundColor(match scroll_mode {
            ScrollMode::Live => Color::Green,
            ScrollMode::Scrollback => Color::Yellow,
        }),
        Print(right),
        ResetColor,
    )?;

    Ok(())
}

fn clear_line(w: &mut impl Write, row: u16) -> io::Result<()> {
    queue!(w, MoveTo(0, row), Clear(ClearType::CurrentLine))
}

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

fn build_right(state: &SystemState, col: u16) -> Vec<Line> {
    const KEY_WIDTH: u16 = 3;
    let val_col = col.saturating_add(KEY_WIDTH).saturating_add(1);
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
        ip_line.push(Span::new(
            Color::White,
            format!("{:<width$} ", "IP", width = usize::from(KEY_WIDTH)),
        ));
        ip_line.push(Span::reset(
            all_addrs.first().copied().unwrap_or_default().to_owned(),
        ));
        lines.push(ip_line);

        for addr in all_addrs.iter().skip(1).take(2) {
            let mut cont = Line::new(val_col);
            cont.push(Span::reset((*addr).to_owned()));
            lines.push(cont);
        }
    }

    if let Some(gw) = state.gateway.as_deref() {
        lines.push(net_kv_line(col, "GW", gw));
    }

    if !state.dns_servers.is_empty() {
        lines.push(net_kv_line(col, "DNS", &state.dns_servers.join(", ")));
    }

    if let Some(ntp) = state.ntp_server.as_deref() {
        lines.push(net_kv_line(col, "NTP", ntp));
    }

    lines
}

fn net_kv_line(col: u16, key: &str, val: &str) -> Line {
    let mut line = Line::new(col);
    line.push(Span::new(Color::White, format!("{key:<3} ")));
    line.push(Span::reset(val.to_owned()));

    line
}

fn format_gib(total_kb: u64) -> String {
    const GIB_IN_KB: u64 = 1024 * 1024;
    let mut whole = total_kb.div_euclid(GIB_IN_KB);
    let mut fraction = total_kb
        .rem_euclid(GIB_IN_KB)
        .wrapping_mul(10)
        .wrapping_mul(2)
        .wrapping_add(GIB_IN_KB)
        .div_euclid(GIB_IN_KB.wrapping_mul(2));
    if fraction >= 10 {
        fraction = fraction.rem_euclid(10);
        whole = whole.saturating_add(1);
    }

    format!("{whole}.{fraction}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::{CpuUsage, MemoryInfo, NetInterface, SystemStatus, Uptime};

    fn test_state() -> SystemState {
        SystemState {
            hostname: "muak-test".to_owned(),
            version: "0.1.0".to_owned(),
            uptime: Uptime::default(),
            cpu: CpuUsage::default(),
            memory: MemoryInfo::default(),
            system_status: SystemStatus::Maintenance,
            secure_boot: false,
            ntp_server: None,
            interfaces: vec![NetInterface {
                name: "eth0".to_owned(),
                addresses: vec!["192.168.1.5".to_owned()],
            }],
            gateway: None,
            dns_servers: Vec::new(),
        }
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

        // ASSERT
        let ip_rows = lines
            .iter()
            .take(3)
            .filter(|line| !line.spans.is_empty())
            .count();
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
        assert!(
            lines
                .first()
                .is_some_and(|line| line.spans.iter().any(|span| span.text.contains("none")))
        );
    }

    #[test]
    fn format_gib_rounds_to_one_decimal() {
        // ARRANGE
        let kib = 1024 * 1024;

        // ACT / ASSERT
        assert_eq!(format_gib(kib), "1.0");
        assert_eq!(format_gib(kib * 2), "2.0");
        assert_eq!(format_gib(0), "0.0");
    }
}
