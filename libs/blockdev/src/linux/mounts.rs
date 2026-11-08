//! Kernel swap table reading.

use std::fs;
use std::io::ErrorKind;
use std::path::Path;

use crate::error::{BlockdevError, Result};
use crate::mounts::Swap;

/// Reads the swap table at `path`.
///
/// # Errors
///
/// Returns an error when the table exists but cannot be read. A missing table
/// yields an empty list.
pub(crate) fn try_read_swaps(path: &Path) -> Result<Vec<Swap>> {
    match fs::read_to_string(path) {
        Ok(content) => Ok(parse_swaps(&content)),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(Vec::new()),
        Err(error) => Err(BlockdevError::Io(error)),
    }
}

fn parse_swaps(content: &str) -> Vec<Swap> {
    content
        .lines()
        .skip(1)
        .filter_map(|line| line.split_whitespace().next())
        .map(|device| Swap {
            device: device.to_owned(),
        })
        .collect()
}
