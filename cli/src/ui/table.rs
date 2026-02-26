//! Table renderer with styled headers and auto-width columns.

/// A simple table renderer.
pub struct Table {
    headers: Vec<String>,
    rows: Vec<Vec<String>>,
    widths: Vec<usize>,
}

impl Default for Table {
    fn default() -> Self {
        Self::new()
    }
}

impl Table {
    /// Creates a new empty table.
    pub fn new() -> Self {
        Self {
            headers: Vec::new(),
            rows: Vec::new(),
            widths: Vec::new(),
        }
    }

    /// Sets the header row.
    pub fn header(mut self, columns: &[&str]) -> Self {
        self.headers = columns.iter().map(|s| s.to_string()).collect();
        self.widths = columns.iter().map(|s| s.len()).collect();
        self
    }

    /// Adds a data row.
    pub fn row(mut self, columns: &[&str]) -> Self {
        let cells: Vec<String> = columns.iter().map(|s| s.to_string()).collect();

        for (i, cell) in cells.iter().enumerate() {
            if i < self.widths.len() {
                self.widths[i] = self.widths[i].max(cell.len());
            } else {
                self.widths.push(cell.len());
            }
        }

        self.rows.push(cells);
        self
    }

    /// Adds a sub-row with a prefix.
    pub fn sub_row(mut self, prefix: &str, columns: &[&str]) -> Self {
        let mut cells: Vec<String> = columns.iter().map(|s| s.to_string()).collect();
        if let Some(first) = cells.first_mut() {
            *first = format!("  {prefix} {first}");
        }

        for (i, cell) in cells.iter().enumerate() {
            if i < self.widths.len() {
                self.widths[i] = self.widths[i].max(cell.len());
            } else {
                self.widths.push(cell.len());
            }
        }

        self.rows.push(cells);
        self
    }

    /// Prints the table to stdout.
    pub fn print(self) {
        if !self.headers.is_empty() {
            let header_line = self.format_row(&self.headers);
            println!("{}", super::style::header(&header_line));
        }

        for row in &self.rows {
            println!("{}", self.format_row(row));
        }
    }

    /// Formats a row with padded columns.
    fn format_row(&self, cells: &[String]) -> String {
        let mut parts = Vec::new();
        let last_idx = cells.len().saturating_sub(1);

        for (i, cell) in cells.iter().enumerate() {
            if i == last_idx {
                parts.push(cell.clone());
            } else {
                let width = self.widths.get(i).copied().unwrap_or(0);
                parts.push(format!("{:<width$}", cell, width = width + 2));
            }
        }

        parts.join("")
    }
}
