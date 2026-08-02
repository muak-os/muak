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
        self.headers = columns.iter().copied().map(str::to_owned).collect();
        self.widths = columns.iter().map(|col| col.len()).collect();
        self
    }

    /// Adds a data row.
    #[must_use]
    pub fn row(mut self, columns: &[&str]) -> Self {
        let cells: Vec<String> = columns.iter().copied().map(str::to_owned).collect();

        for (i, cell) in cells.iter().enumerate() {
            match self.widths.get_mut(i) {
                Some(width) => *width = (*width).max(cell.len()),
                None => self.widths.push(cell.len()),
            }
        }

        self.rows.push(cells);
        self
    }

    /// Adds a sub-row with a prefix.
    #[must_use]
    pub fn sub_row(mut self, prefix: &str, columns: &[&str]) -> Self {
        let mut cells: Vec<String> = columns.iter().copied().map(str::to_owned).collect();
        if let Some(first) = cells.first_mut() {
            *first = format!("  {prefix} {first}");
        }

        for (i, cell) in cells.iter().enumerate() {
            match self.widths.get_mut(i) {
                Some(width) => *width = (*width).max(cell.len()),
                None => self.widths.push(cell.len()),
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

    fn format_row(&self, cells: &[String]) -> String {
        let mut parts = Vec::new();

        for (i, cell) in cells.iter().enumerate() {
            parts.push(self.format_cell(i, cell, i == cells.len().saturating_sub(1)));
        }

        parts.join("")
    }

    fn format_cell(&self, i: usize, cell: &str, is_last: bool) -> String {
        if is_last {
            cell.to_owned()
        } else {
            let width = self.widths.get(i).copied().unwrap_or(0);
            format!("{:<width$}", cell, width = width.saturating_add(2))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_is_empty() {
        // ARRANGE & ACT
        let table = Table::new();

        // ASSERT
        assert!(table.headers.is_empty());
        assert!(table.rows.is_empty());
        assert!(table.widths.is_empty());
    }

    #[test]
    fn default_equals_new() {
        let table = Table::default();
        assert!(table.headers.is_empty());
    }

    #[test]
    fn header_sets_widths() {
        // ARRANGE & ACT
        let table = Table::new().header(&["Name", "Status"]);

        // ASSERT
        assert_eq!(table.widths, vec![4, 6]);
    }

    #[test]
    fn row_expands_widths() {
        // ARRANGE & ACT
        let table = Table::new()
            .header(&["Name", "Status"])
            .row(&["a-very-long-name", "running"]);

        // ASSERT
        assert_eq!(table.widths.first(), Some(&16));
        assert_eq!(table.widths.get(1), Some(&7));
    }

    #[test]
    fn format_row_pads_all_but_last() {
        // ARRANGE & ACT
        let table = Table::new().header(&["AA", "BB"]).row(&["x", "y"]);
        let row = table.format_row(&["x".to_owned(), "y".to_owned()]);

        // ASSERT
        assert_eq!(row, "x   y");
    }

    #[test]
    fn format_row_single_cell_no_padding() {
        let table = Table::new().header(&["Col"]);
        let row = table.format_row(&["val".to_owned()]);
        assert_eq!(row, "val");
    }

    #[test]
    fn sub_row_prepends_prefix_to_first_cell() {
        // ARRANGE & ACT
        let table = Table::new()
            .header(&["Name", "Status"])
            .sub_row("└", &["child", "ok"]);

        // ASSERT
        assert_eq!(
            table
                .rows
                .first()
                .and_then(|row| row.first())
                .map(String::as_str),
            Some("  └ child")
        );
    }

    #[test]
    fn sub_row_expands_width_for_prefixed_cell() {
        // ARRANGE & ACT
        let table = Table::new()
            .header(&["N", "S"])
            .sub_row("→", &["child", "ok"]);

        // ASSERT
        assert_eq!(table.widths.first(), Some(&"  → child".len()));
    }

    #[test]
    fn row_without_prior_header_grows_widths() {
        // ARRANGE & ACT
        let table = Table::new().row(&["hello", "world"]);

        // ASSERT
        assert_eq!(table.widths, vec![5, 5]);
    }
}
