//! Terminal surface dimensions.

#[cfg(feature = "size")]
use std::os::fd::AsFd;

#[cfg(feature = "size")]
use rustix::termios::tcgetwinsize;

/// Terminal surface dimensions.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Size {
    /// Number of columns.
    pub cols: u16,
    /// Number of rows.
    pub rows: u16,
}

impl Size {
    /// Creates a size from `cols` and `rows`.
    #[must_use]
    pub const fn new(cols: u16, rows: u16) -> Self {
        Self { cols, rows }
    }
}

impl Default for Size {
    fn default() -> Self {
        Self::new(80, 25)
    }
}

/// Returns the size of the terminal attached to `fd`.
#[cfg(feature = "size")]
#[must_use]
pub fn of<Fd: AsFd>(fd: Fd) -> Size {
    tcgetwinsize(fd).map_or_else(
        |_error| Size::default(),
        |winsize| Size::new(winsize.ws_col, winsize.ws_row),
    )
}

/// Returns the size of the terminal attached to standard output.
#[cfg(feature = "size")]
#[must_use]
pub fn stdout() -> Size {
    of(rustix::stdio::stdout())
}

/// Returns the size of the terminal attached to standard error.
#[cfg(feature = "size")]
#[must_use]
pub fn stderr() -> Size {
    of(rustix::stdio::stderr())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_size_is_eighty_by_twenty_five() {
        // ARRANGE
        let expected = Size::new(80, 25);

        // ACT
        let size = Size::default();

        // ASSERT
        assert_eq!(size, expected);
    }

    #[test]
    fn new_stores_columns_and_rows() {
        // ARRANGE
        let cols = 120;
        let rows = 40;

        // ACT
        let size = Size::new(cols, rows);

        // ASSERT
        assert_eq!(size.cols, cols);
        assert_eq!(size.rows, rows);
    }
}
