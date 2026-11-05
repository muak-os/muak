//! Device-mapper holders of a block device.

use std::fs;
use std::io::ErrorKind;

use crate::error::{BlockdevError, Result};
use crate::roots::Roots;

/// Returns the device-mapper names of the holders of `device`.
///
/// # Errors
///
/// Returns an error when the holders directory exists but cannot be read. A
/// device without a holders directory yields an empty list.
pub fn try_names(roots: &Roots, device: &str) -> Result<Vec<String>> {
    let directory = roots.block.join(device).join("holders");
    let entries = match fs::read_dir(&directory) {
        Ok(entries) => entries,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(BlockdevError::Io(error)),
    };

    let mut names: Vec<String> = entries
        .flatten()
        .map(|entry| mapped(roots, &entry.file_name().to_string_lossy()))
        .collect();

    names.sort();
    names.dedup();

    Ok(names)
}

fn mapped(roots: &Roots, holder: &str) -> String {
    match fs::read_to_string(roots.block.join(holder).join("dm/name")) {
        Ok(name) => name.trim().to_owned(),
        Err(_error) => holder.to_owned(),
    }
}
