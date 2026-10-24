//! Table rendering with styled cells and display widths.

use alloc::borrow::Cow;
use std::io;

use crate::screen::{Ansi, Screen};
use crate::size::Size;
use crate::style::{Role, Style, Theme};
use crate::text::{self, Span};

/// A table cell composed of styled spans.
pub struct Cell {
    spans: Vec<Span<'static>>,
}

/// A table with styled headers and automatic column widths.
pub struct Table {
    headers: Vec<Cell>,
    rows: Vec<Vec<Cell>>,
    widths: Vec<usize>,
}

impl Cell {
    /// Creates a plain cell with sanitized text.
    #[must_use]
    pub fn plain<T: Into<String>>(text: T) -> Self {
        let text = text::sanitize(Cow::Owned(text.into())).into_owned();

        Self::from_spans(vec![Span::plain(text)])
    }

    /// Creates a styled cell with sanitized text.
    #[must_use]
    pub fn styled<T: Into<String>>(text: T, style: Style) -> Self {
        let text = text::sanitize(Cow::Owned(text.into())).into_owned();

        Self::from_spans(vec![Span::styled(text, style)])
    }

    /// Creates a cell from existing spans.
    #[must_use]
    pub fn from_spans(spans: Vec<Span<'static>>) -> Self {
        Self { spans }
    }

    /// Returns the display width of the cell.
    #[must_use]
    pub fn width(&self) -> usize {
        self.spans
            .iter()
            .fold(0, |total, span| total.saturating_add(span.width()))
    }
}

impl Default for Table {
    fn default() -> Self {
        Self::new()
    }
}

impl Table {
    /// Creates an empty table.
    #[must_use]
    pub fn new() -> Self {
        Self {
            headers: Vec::new(),
            rows: Vec::new(),
            widths: Vec::new(),
        }
    }

    /// Sets the header row.
    #[must_use]
    pub fn header(mut self, columns: &[&str]) -> Self {
        self.headers = columns.iter().map(|column| Cell::plain(*column)).collect();
        self.widths = self.headers.iter().map(Cell::width).collect();

        self
    }

    /// Adds a data row.
    #[must_use]
    pub fn row(mut self, columns: &[&str]) -> Self {
        let cells: Vec<Cell> = columns.iter().map(|column| Cell::plain(*column)).collect();
        self.include(&cells);
        self.rows.push(cells);

        self
    }

    /// Adds a data row from owned cells.
    #[must_use]
    pub fn row_cells(mut self, cells: Vec<Cell>) -> Self {
        self.include(&cells);
        self.rows.push(cells);

        self
    }

    /// Adds a row whose first cell is prefixed with `prefix`.
    #[must_use]
    pub fn sub_row(mut self, prefix: &str, columns: &[&str]) -> Self {
        let mut cells: Vec<Cell> = columns.iter().map(|column| Cell::plain(*column)).collect();

        if let Some(first) = cells.first_mut() {
            first.spans.insert(0, Span::plain(format!("  {prefix} ")));
        }

        self.include(&cells);
        self.rows.push(cells);

        self
    }

    /// Renders the table inline through `screen`.
    ///
    /// # Errors
    ///
    /// Returns an error when the screen writer fails.
    pub fn render<S: Screen>(&self, screen: &mut S, theme: Theme) -> io::Result<()> {
        if !self.headers.is_empty() {
            self.render_row(screen, &self.headers, theme, true)?;
        }

        for row in &self.rows {
            self.render_row(screen, row, theme, false)?;
        }

        screen.flush()
    }

    /// Renders the table to standard output.
    ///
    /// # Errors
    ///
    /// Returns an error when standard output fails.
    pub fn print(&self) -> io::Result<()> {
        let mut screen = Ansi::new(std::io::stdout(), Size::default());

        self.render(&mut screen, crate::style::stdout())
    }

    fn include(&mut self, cells: &[Cell]) {
        for (index, cell) in cells.iter().enumerate() {
            let width = cell.width();
            match self.widths.get_mut(index) {
                Some(existing) => *existing = (*existing).max(width),
                None => self.widths.push(width),
            }
        }
    }

    fn render_row<S: Screen>(
        &self,
        screen: &mut S,
        cells: &[Cell],
        theme: Theme,
        header: bool,
    ) -> io::Result<()> {
        if let Some((last, rest)) = cells.split_last() {
            self.render_cells(screen, rest, theme, header)?;
            write_cell(screen, last, theme, header)?;
        }

        screen.newline()
    }

    fn render_cells<S: Screen>(
        &self,
        screen: &mut S,
        cells: &[Cell],
        theme: Theme,
        header: bool,
    ) -> io::Result<()> {
        for (index, cell) in cells.iter().enumerate() {
            write_cell(screen, cell, theme, header)?;
            self.write_padding(screen, index, cell)?;
        }

        Ok(())
    }

    fn write_padding<S: Screen>(
        &self,
        screen: &mut S,
        index: usize,
        cell: &Cell,
    ) -> io::Result<()> {
        let column_width = self
            .widths
            .get(index)
            .copied()
            .unwrap_or_else(|| cell.width());
        let padding = column_width.saturating_sub(cell.width()).saturating_add(2);

        write_spaces(screen, padding)
    }
}

fn write_cell<S: Screen>(
    screen: &mut S,
    cell: &Cell,
    theme: Theme,
    header: bool,
) -> io::Result<()> {
    if header {
        return write_header_cell(screen, cell, theme);
    }

    for span in &cell.spans {
        screen.write_span(span)?;
    }

    Ok(())
}

fn write_header_cell<S: Screen>(screen: &mut S, cell: &Cell, theme: Theme) -> io::Result<()> {
    for span in &cell.spans {
        let styled = Span::styled(span.text().to_owned(), theme.style(Role::Header));
        screen.write_span(&styled)?;
    }

    Ok(())
}

fn write_spaces<S: Screen>(screen: &mut S, count: usize) -> io::Result<()> {
    let spaces = " ".repeat(count);

    screen.write_span(&Span::plain(spaces))
}
