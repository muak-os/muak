//! Row and cell rendering shared by natural and fitted tables.

use std::io;

use crate::screen::Screen;
use crate::style::{Role, Style, Theme};
use crate::table::Cell;
use crate::text::{self, Span};

pub(crate) fn row<S: Screen>(
    screen: &mut S,
    cells: &[Cell],
    widths: &[usize],
    theme: Theme,
    header: bool,
) -> io::Result<()> {
    if let Some((last, rest)) = cells.split_last() {
        columns(screen, rest, widths, theme, header)?;
        cell(screen, last, widths.get(rest.len()).copied(), theme, header)?;
    }

    screen.newline()
}

fn columns<S: Screen>(
    screen: &mut S,
    cells: &[Cell],
    widths: &[usize],
    theme: Theme,
    header: bool,
) -> io::Result<()> {
    for (index, item) in cells.iter().enumerate() {
        let column_width = widths.get(index).copied();
        let rendered = cell(screen, item, column_width, theme, header)?;
        padding(screen, column_width.unwrap_or(rendered), rendered)?;
    }

    Ok(())
}

fn padding<S: Screen>(screen: &mut S, column_width: usize, rendered: usize) -> io::Result<()> {
    spaces(
        screen,
        column_width.saturating_sub(rendered).saturating_add(2),
    )
}

fn cell<S: Screen>(
    screen: &mut S,
    cell: &Cell,
    width: Option<usize>,
    theme: Theme,
    header: bool,
) -> io::Result<usize> {
    if let Some(width) = width
        && cell.width() > width
    {
        return elided(screen, cell, width, theme, header);
    }

    if header {
        return header_cell(screen, cell, theme);
    }

    for span in &cell.spans {
        screen.write_span(span)?;
    }

    Ok(cell.width())
}

fn header_cell<S: Screen>(screen: &mut S, cell: &Cell, theme: Theme) -> io::Result<usize> {
    for span in &cell.spans {
        let styled = Span::styled(span.text().to_owned(), theme.style(Role::Header));
        screen.write_span(&styled)?;
    }

    Ok(cell.width())
}

fn elided<S: Screen>(
    screen: &mut S,
    cell: &Cell,
    width: usize,
    theme: Theme,
    header: bool,
) -> io::Result<usize> {
    let mut text = String::new();
    for span in &cell.spans {
        text.push_str(span.text());
    }
    let elided = text::elide(&text, width);
    let style = if header {
        theme.style(Role::Header)
    } else {
        cell.spans.first().map_or_else(Style::default, Span::style)
    };
    let span = Span::styled(elided, style);
    screen.write_span(&span)?;

    Ok(text::width(span.text()))
}

fn spaces<S: Screen>(screen: &mut S, count: usize) -> io::Result<()> {
    let spaces = " ".repeat(count);

    screen.write_span(&Span::plain(spaces))
}
