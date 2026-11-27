//! Sysfs attribute readers and the single GPT name parser of the workspace.

use std::fs;
use std::path::Path;

use parttable::gpt;

/// Returns the value of the first `PARTNAME=` line of a `uevent` file.
pub(crate) fn partname(content: &str) -> String {
    content
        .lines()
        .find_map(|line| line.trim().strip_prefix("PARTNAME="))
        .map_or_else(String::new, str::to_owned)
}

/// Reads a trimmed integer value from a sysfs attribute file.
pub(crate) fn read_u64(path: &Path) -> Option<u64> {
    read_string(path)?.parse().ok()
}

/// Reads a sysfs `size` attribute, converting its 512-byte sectors to bytes.
pub(crate) fn read_size_bytes(path: &Path) -> Option<u64> {
    read_u64(path)?.checked_mul(gpt::plan::SECTOR_SIZE)
}

/// Reads a trimmed string value from a sysfs attribute file.
pub(crate) fn read_string(path: &Path) -> Option<String> {
    let content = fs::read_to_string(path).ok()?;

    Some(content.trim().to_owned())
}

/// Reads the file name of a symlink target, keeping the link unresolved.
pub(crate) fn read_link_name(path: &Path) -> Option<String> {
    let target = fs::read_link(path).ok()?;

    Some(target.file_name()?.to_string_lossy().into_owned())
}
