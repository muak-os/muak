//! Table rendering with styled cells and display widths.

mod fit;
mod render;

use alloc::borrow::Cow;

use crate::screen::Screen;
use crate::screen::ansi::Ansi;
use crate::size::Size;
use crate::style::{Style, Theme};
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

    /// Renders the table inline through `screen` at its natural width.
    ///
    /// # Errors
    ///
    /// Returns an error when the screen writer fails.
    pub fn render<S: Screen>(&self, screen: &mut S, theme: Theme) -> std::io::Result<()> {
        self.write(screen, theme, None)
    }

    /// Renders the table inline through `screen`, fitting it to `cols` columns.
    ///
    /// Cells wider than their fitted column are elided to their flattened text,
    /// keeping the style of their first span.
    ///
    /// # Errors
    ///
    /// Returns an error when the screen writer fails.
    pub fn render_fitted<S: Screen>(
        &self,
        screen: &mut S,
        theme: Theme,
        cols: u16,
    ) -> std::io::Result<()> {
        self.write(screen, theme, Some(usize::from(cols)))
    }

    /// Renders the table to standard output.
    ///
    /// # Errors
    ///
    /// Returns an error when standard output fails.
    pub fn print(&self) -> std::io::Result<()> {
        let mut screen = Ansi::new(std::io::stdout(), Size::default());

        self.render(&mut screen, crate::style::stdout())
    }

    fn write<S: Screen>(
        &self,
        screen: &mut S,
        theme: Theme,
        cols: Option<usize>,
    ) -> std::io::Result<()> {
        match cols {
            Some(cols) => {
                let widths = fit::widths(&self.headers, &self.widths, cols);
                self.write_rows(screen, &widths, theme)
            }
            None => self.write_rows(screen, &self.widths, theme),
        }
    }

    fn write_rows<S: Screen>(
        &self,
        screen: &mut S,
        widths: &[usize],
        theme: Theme,
    ) -> std::io::Result<()> {
        if !self.headers.is_empty() {
            render::row(screen, &self.headers, widths, theme, true)?;
        }

        for row in &self.rows {
            render::row(screen, row, widths, theme, false)?;
        }

        screen.flush()
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
}
