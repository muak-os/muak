//! Header drawing of the summary status line.

use std::io;

use tui::screen::Screen;
use tui::style::{AnsiColor, bold_fg};
use tui::text::{Line, Span};
use tui::units;

use crate::state::SystemState;

/// Draws the summary header line and returns the next row.
pub(super) fn draw<S: Screen>(screen: &mut S, state: &SystemState, row: u16) -> io::Result<u16> {
    let uptime = &state.uptime;
    let total = units::bytes(state.memory.total_kb.saturating_mul(1024));

    let cpu_permille = state.cpu.permille;
    let ram_permille = state.memory.permille();
    let summary = format!(
        "up {}d {}h {}m, {total} RAM, CPU {}.{}%, RAM {}.{}%",
        uptime.days,
        uptime.hours,
        uptime.minutes,
        cpu_permille.div_euclid(10),
        cpu_permille.rem_euclid(10),
        ram_permille.div_euclid(10),
        ram_permille.rem_euclid(10),
    );

    let mut line = Line::new(0);
    line.push(Span::styled(
        format!("  {}", state.hostname),
        bold_fg(AnsiColor::Cyan),
    ));
    if !state.version.is_empty() {
        line.push(Span::plain(format!(" ({})", state.version)));
    }
    line.push(Span::plain(": "));
    line.push(Span::plain(summary));

    screen.clear_line(row)?;
    screen.write_line(row, &line)?;

    Ok(row.saturating_add(1))
}

#[cfg(test)]
mod tests {
    use firmware::secureboot::Status;
    use tui::screen::Ansi;
    use tui::size::Size;

    use super::*;
    use crate::state::{CpuUsage, MemoryInfo, SystemStatus, Uptime};

    #[test]
    fn version_is_rendered_without_an_extra_prefix() {
        // ARRANGE
        let state = SystemState {
            hostname: "muak".to_owned(),
            version: "v1.1.0-nightly.2026-10-30.gce1327e3".to_owned(),
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
        let mut screen = Ansi::new(Vec::new(), Size::new(200, 1));

        // ACT
        draw(&mut screen, &state, 0).unwrap();

        // ASSERT
        let output = String::from_utf8_lossy(&screen.into_inner()).into_owned();
        assert!(output.contains("(v1.1.0-nightly.2026-10-30.gce1327e3)"));
        assert!(!output.contains("vv1.1.0-nightly.2026-10-30.gce1327e3"));
    }
}
