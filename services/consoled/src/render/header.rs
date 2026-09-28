//! Header drawing of the summary status line.

use std::io::Write;

use anyhow::Result;
use crossterm::cursor::MoveTo;
use crossterm::queue;
use crossterm::style::{Attribute, Color, Print, ResetColor, SetAttribute, SetForegroundColor};

use super::clear_line;
use crate::state::SystemState;

/// Draws the summary header line and returns the next row.
pub(super) fn draw(w: &mut impl Write, state: &SystemState, row: u16) -> Result<u16> {
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
