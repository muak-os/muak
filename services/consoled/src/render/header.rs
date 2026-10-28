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
    line.push(Span::plain(format!(" (v{})", state.version)));
    line.push(Span::plain(": "));
    line.push(Span::plain(summary));

    screen.clear_line(row)?;
    screen.write_line(row, &line)?;

    Ok(row.saturating_add(1))
}
