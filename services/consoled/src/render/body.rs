//! Body drawing of the two-column system status.

use std::io;

use firmware::secureboot::{Mismatch, Status};
use tui::screen::Screen;
use tui::style::{AnsiColor, bold_fg, fg};
use tui::text::{Line, Span};

use super::PANEL_BODY_ROWS;
use crate::state::{SystemState, SystemStatus};

/// Draws the two-column status body between the separators.
pub(super) fn draw<S: Screen>(
    screen: &mut S,
    state: &SystemState,
    cols: u16,
    start_row: u16,
) -> io::Result<()> {
    let mid_col = cols.div_euclid(2);

    let left_lines = build_left(state, 2);
    let right_lines = build_right(state, mid_col);
    let empty_line = Line::new(0);

    for i in 0..usize::from(PANEL_BODY_ROWS) {
        let row = start_row.saturating_add(u16::try_from(i).unwrap_or(0));
        screen.clear_line(row)?;
        screen.write_line(row, left_lines.get(i).unwrap_or(&empty_line))?;
        screen.write_line(row, right_lines.get(i).unwrap_or(&empty_line))?;
    }

    Ok(())
}

fn build_left(state: &SystemState, col: u16) -> Vec<Line<'static>> {
    let (status_label, status_color) = match state.system_status {
        SystemStatus::Installed => ("INSTALLED", AnsiColor::Green),
        SystemStatus::Maintenance => ("MAINTENANCE", AnsiColor::Red),
    };

    let (sb_label, sb_color) = match state.secure_boot_mismatch {
        Some(_) => (short_status(state.secure_boot), AnsiColor::Yellow),
        None => firmware_label(state.secure_boot),
    };

    let mut lines = Vec::new();

    let mut status_line = Line::new(col);
    status_line.push(Span::styled("STATUS     ", fg(AnsiColor::White)));
    status_line.push(Span::styled(status_label, bold_fg(status_color)));
    lines.push(status_line);

    let mut sb_line = Line::new(col);
    sb_line.push(Span::styled("SECUREBOOT ", fg(AnsiColor::White)));
    sb_line.push(Span::styled(sb_label, bold_fg(sb_color)));
    if let Some(mismatch) = state.secure_boot_mismatch {
        let note = mismatch_note(mismatch);
        sb_line.push(Span::styled(format!(" ({note})"), fg(AnsiColor::Yellow)));
    }
    lines.push(sb_line);

    lines
}

fn firmware_label(status: Status) -> (&'static str, AnsiColor) {
    match status {
        Status::Enabled => ("Enabled", AnsiColor::Green),
        Status::Pending => ("Pending (firmware reboot required)", AnsiColor::Yellow),
        Status::Disabled => ("Disabled", AnsiColor::Red),
    }
}

fn short_status(status: Status) -> &'static str {
    match status {
        Status::Enabled => "Enabled",
        Status::Pending => "Pending",
        Status::Disabled => "Disabled",
    }
}

fn mismatch_note(mismatch: Mismatch) -> &'static str {
    match mismatch {
        Mismatch::NotEnrolled => "no keys",
        Mismatch::KeysRemain => "keys remain",
        Mismatch::EnforcingWhileDisabled => "still enforced",
    }
}

fn build_right(state: &SystemState, col: u16) -> Vec<Line<'static>> {
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
        ip_line.push(Span::styled(
            format!("{:<width$} ", "IP", width = usize::from(KEY_WIDTH)),
            fg(AnsiColor::White),
        ));
        ip_line.push(Span::plain(
            all_addrs.first().copied().unwrap_or_default().to_owned(),
        ));
        lines.push(ip_line);

        for addr in all_addrs.iter().skip(1).take(2) {
            let mut cont = Line::new(val_col);
            cont.push(Span::plain((*addr).to_owned()));
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

fn net_kv_line(col: u16, key: &str, val: &str) -> Line<'static> {
    let mut line = Line::new(col);
    line.push(Span::styled(format!("{key:<3} "), fg(AnsiColor::White)));
    line.push(Span::plain(val.to_owned()));

    line
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::{CpuUsage, MemoryInfo, NetInterface, Uptime};

    fn test_state() -> SystemState {
        SystemState {
            hostname: "muak-test".to_owned(),
            version: "0.1.0".to_owned(),
            uptime: Uptime::default(),
            cpu: CpuUsage::default(),
            memory: MemoryInfo::default(),
            system_status: SystemStatus::Maintenance,
            secure_boot: Status::Disabled,
            secure_boot_mismatch: None,
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
        state.secure_boot = Status::Enabled;

        // ACT
        let lines = build_left(&state, 0);

        // ASSERT
        assert_eq!(lines.len(), 2);
    }

    #[test]
    fn build_left_secureboot_pending_label() {
        // ARRANGE
        let mut state = test_state();
        state.secure_boot = Status::Pending;

        // ACT
        let lines = build_left(&state, 0);

        // ASSERT
        let Some(sb_line) = lines.get(1) else {
            panic!("expected SECUREBOOT line, got: {lines:?}");
        };
        let pending = sb_line
            .spans()
            .iter()
            .any(|span| span.text() == "Pending (firmware reboot required)");
        assert!(
            pending,
            "expected pending label, got: {:?}",
            sb_line.spans()
        );
    }

    #[test]
    fn build_left_secureboot_mismatch_shows_note() {
        // ARRANGE
        let mut state = test_state();
        state.secure_boot = Status::Enabled;
        state.secure_boot_mismatch = Some(Mismatch::EnforcingWhileDisabled);

        // ACT
        let lines = build_left(&state, 0);

        // ASSERT
        let Some(sb_line) = lines.get(1) else {
            panic!("expected SECUREBOOT line, got: {lines:?}");
        };
        let note = sb_line
            .spans()
            .iter()
            .any(|span| span.text() == " (still enforced)");
        assert!(note, "expected mismatch note, got: {:?}", sb_line.spans());
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
            .filter(|line| !line.spans().is_empty())
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
                .is_some_and(|line| line.spans().iter().any(|span| span.text().contains("none")))
        );
    }
}
