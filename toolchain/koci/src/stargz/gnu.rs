//! GNU tar long-name extension encoding for repacked layers.

use std::ffi::OsStr;
use std::os::unix::ffi::OsStrExt as _;
use std::path::Path;

use tar::{EntryType, Header};

use crate::error::{KociError, Result};

/// Name carried by every GNU long-name and long-link extension entry.
const GNU_LONG_NAME: &str = "././@LongLink";

/// Entry type byte of the GNU long-name extension.
const LONG_NAME: u8 = b'L';

/// Entry type byte of the GNU long-link extension.
const LONG_LINK: u8 = b'K';

/// Tar block size in bytes.
const BLOCK: usize = 512;

/// Longest path stored inline in a GNU header name field.
const NAME_FIELD: usize = 100;

/// Sets `path` on `header`, returning extension bytes when it does not fit.
///
/// # Errors
///
/// Returns an error when the fallback truncated path cannot be stored.
pub(crate) fn long_name(header: &mut Header, path: &[u8]) -> Result<Vec<u8>> {
    if header.set_path(Path::new(OsStr::from_bytes(path))).is_ok() {
        return Ok(Vec::new());
    }

    let prefix = extension(EntryType::new(LONG_NAME), path)?;
    let fallback = tail(path, NAME_FIELD);
    header
        .set_path(Path::new(OsStr::from_bytes(fallback)))
        .map_err(|error| KociError::PushError(format!("set truncated path: {error}")))?;

    Ok(prefix)
}

/// Sets `link` on `header`, returning extension bytes when it does not fit.
///
/// # Errors
///
/// Returns an error when the fallback truncated link cannot be stored.
pub(crate) fn long_link(header: &mut Header, link: &[u8]) -> Result<Vec<u8>> {
    if header
        .set_link_name(Path::new(OsStr::from_bytes(link)))
        .is_ok()
    {
        return Ok(Vec::new());
    }

    let prefix = extension(EntryType::new(LONG_LINK), link)?;
    let fallback = tail(link, NAME_FIELD);
    header
        .set_link_name(Path::new(OsStr::from_bytes(fallback)))
        .map_err(|error| KociError::PushError(format!("set truncated link: {error}")))?;

    Ok(prefix)
}

fn extension(kind: EntryType, value: &[u8]) -> Result<Vec<u8>> {
    let payload = value.len().checked_add(1).ok_or_else(overflow)?;
    let mut header = Header::new_gnu();
    header.set_entry_type(kind);
    header.set_size(
        u64::try_from(payload)
            .map_err(|error| KociError::PushError(format!("long-name size overflows: {error}")))?,
    );
    header.set_mode(0o644);
    header.set_uid(0);
    header.set_gid(0);
    header.set_mtime(0);
    header
        .set_path(GNU_LONG_NAME)
        .map_err(|error| KociError::PushError(format!("set extension path: {error}")))?;
    header.set_cksum();

    let mut bytes = Vec::new();
    bytes.extend_from_slice(header.as_bytes());
    bytes.extend_from_slice(value);
    bytes.push(0);
    let aligned = bytes
        .len()
        .checked_next_multiple_of(BLOCK)
        .ok_or_else(overflow)?;
    bytes.resize(aligned, 0);

    Ok(bytes)
}

fn tail(value: &[u8], limit: usize) -> &[u8] {
    let start = value.len().saturating_sub(limit);

    value.get(start..).unwrap_or(value)
}

fn overflow() -> KociError {
    KociError::PushError("long-name extension length overflows".to_owned())
}
