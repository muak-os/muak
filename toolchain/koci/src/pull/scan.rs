//! Scanning and processing OCI layer tar archives.

use std::io::Read;
use std::path::{Path, PathBuf};

use super::entries::FileEntry;
use super::paths::{normalize_entry_path_into, whiteout_target_into};
use crate::error::{KociError, Result};

/// Outcome of classifying a tar entry, borrowing the caller's reused path buffer.
#[derive(Clone, Copy)]
pub(crate) enum EntryInfo<'a> {
    Skip,
    Whiteout(&'a Path),
    File(&'a Path, u64, u32),
}

/// Classify a tar entry into the reused `scratch` buffer.
///
/// # Errors
///
/// Returns an error when the entry path escapes the extraction root or uses an
/// unsupported entry type.
pub(crate) fn classify_tar_entry<'a>(
    entry: &tar::Entry<impl Read>,
    scratch: &'a mut PathBuf,
) -> Result<EntryInfo<'a>> {
    let header = entry.header();
    let entry_type = header.entry_type();

    if !normalize_entry_path_into(entry.path()?.as_ref(), scratch)? {
        return Ok(EntryInfo::Skip);
    }

    if whiteout_target_into(scratch) {
        return Ok(EntryInfo::Whiteout(scratch));
    }

    if entry_type.is_dir() {
        return Ok(EntryInfo::Skip);
    }

    if entry_type.is_symlink() {
        return Err(unsupported_entry("symlink", scratch));
    }

    if entry_type.is_hard_link() {
        return Err(unsupported_entry("hard link", scratch));
    }

    if !entry_type.is_file() {
        return Err(KociError::LayerExtractionError(format!(
            "Unsupported OCI layer entry type for {}",
            scratch.display()
        )));
    }

    Ok(EntryInfo::File(
        scratch,
        header.size().unwrap_or(0),
        header.mode().unwrap_or(0o644),
    ))
}

/// Scan a single layer and collect its whiteout targets.
///
/// # Errors
///
/// Returns an error when an entry cannot be classified.
pub(crate) fn scan_whiteouts<R: Read>(data: R) -> Result<Vec<PathBuf>> {
    let mut whiteouts: Vec<PathBuf> = Vec::new();
    let mut scratch = PathBuf::new();

    let mut archive = tar::Archive::new(data);
    let entries = archive.entries()?;
    for entry_result in entries {
        let entry = entry_result?;
        if let EntryInfo::Whiteout(target) = classify_tar_entry(&entry, &mut scratch)? {
            whiteouts.push(target.to_path_buf());
        }
    }

    Ok(whiteouts)
}

/// Process a single live tar entry for file streaming.
///
/// # Errors
///
/// Returns an error when the handler fails.
pub(crate) fn handle_file_entry<R: Read>(
    mut entry: tar::Entry<R>,
    info: EntryInfo<'_>,
    handler: &mut impl FnMut(FileEntry) -> Result<()>,
) -> Result<()> {
    if let EntryInfo::File(path, size, mode) = info {
        handler(FileEntry {
            path: path_string(path),
            size,
            mode,
            reader: &mut entry,
        })?;
    }

    Ok(())
}

/// The entry path as a `String`, keeping valid UTF-8 exact and falling back to lossy conversion.
pub(crate) fn path_string(path: &Path) -> String {
    match std::str::from_utf8(path.as_os_str().as_encoded_bytes()) {
        Ok(valid) => valid.to_owned(),
        Err(_) => path.to_string_lossy().into_owned(),
    }
}

fn unsupported_entry(kind: &str, path: &Path) -> KociError {
    KociError::LayerExtractionError(format!(
        "Unsupported {kind} entry in OCI layer: {}",
        path.display()
    ))
}

#[cfg(test)]
mod tests {
    use std::io::{Cursor, Read};

    use tar::{Archive, Builder, Header};

    use super::*;

    fn layer_with(entries: &[(&str, &[u8])]) -> Vec<u8> {
        let mut archive = Builder::new(Vec::new());
        for &(path, bytes) in entries {
            let mut header = Header::new_gnu();
            header.set_size(u64::try_from(bytes.len()).expect("size fits u64"));
            header.set_mode(0o644);
            header.set_cksum();
            archive
                .append_data(&mut header, path, bytes)
                .expect("append entry");
        }

        archive.into_inner().expect("finish archive")
    }

    fn classify_one<R: Read>(entry: &tar::Entry<R>, scratch: &mut PathBuf) -> (String, String) {
        match classify_tar_entry(entry, scratch).expect("classify entry") {
            EntryInfo::File(path, ..) => ("file".to_owned(), path_string(path)),
            EntryInfo::Whiteout(path) => {
                ("whiteout".to_owned(), path.to_string_lossy().into_owned())
            }
            EntryInfo::Skip => ("skip".to_owned(), String::new()),
        }
    }

    fn classify_all(layer: &[u8]) -> Vec<(String, String)> {
        let mut results = Vec::new();
        let mut scratch = PathBuf::new();
        let mut archive = Archive::new(Cursor::new(layer));
        for entry in archive.entries().expect("iterate entries") {
            let entry = entry.expect("read entry");
            results.push(classify_one(&entry, &mut scratch));
        }

        results
    }

    #[test]
    fn classify_tar_entry_borrows_a_reused_buffer_without_ownership() {
        // ARRANGE
        let layer = layer_with(&[("etc/motd", b"hello"), ("etc/.wh.gone", b"")]);

        // ACT
        let classified = classify_all(&layer);

        // ASSERT
        assert_eq!(classified.len(), 2);
        let first = classified.first().expect("first entry");
        let second = classified.get(1).expect("second entry");
        assert_eq!(first, &("file".to_owned(), "etc/motd".to_owned()));
        assert_eq!(second, &("whiteout".to_owned(), "etc/gone".to_owned()));
    }
}
