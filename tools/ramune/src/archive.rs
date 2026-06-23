//! Creates a zstd-compressed CPIO archive from a list of entries.

use std::io::{Read, Write};
use std::path::Path;

use crate::compress;
use crate::cpio;
use crate::error::{RamuneError, Result};

/// A single entry in a CPIO archive.
pub struct Entry<'a> {
    /// Destination path inside the archive.
    pub archive_path: &'a Path,
    /// File mode (e.g. `0o100_755` for executables).
    pub mode: u32,
    /// Exact payload length in bytes.
    pub len: u64,
    /// Readable payload stream.
    pub reader: &'a mut dyn Read,
}

impl<'a> Entry<'a> {
    /// Creates an entry from a reader with an explicitly known payload length.
    pub fn new(archive_path: &'a Path, mode: u32, reader: &'a mut dyn Read, len: u64) -> Self {
        Entry {
            archive_path,
            mode,
            len,
            reader,
        }
    }
}

/// Writes a zstd-compressed CPIO archive containing the given entries.
///
/// Entries are sorted by `archive_path` before compression.
///
/// # Errors
///
/// Returns an error when validation fails, an entry exceeds CPIO limits,
/// reading an entry fails, or zstd compression fails.
pub fn archive<W: Write>(
    entries: &mut [Entry],
    compression_level: i32,
    writer: &mut W,
) -> Result<()> {
    entries.sort_unstable_by(|left, right| left.archive_path.cmp(right.archive_path));
    validate_entries(entries)?;

    if entries.is_empty() {
        return Ok(());
    }

    let mut encoder = compress::encoder(writer, compression_level)?;
    let mut ino = 1_u32;

    for entry in entries {
        let archive_path = entry.archive_path.to_string_lossy();
        let size = u32::try_from(entry.len)
            .map_err(|_err| RamuneError::CpioError("file exceeds CPIO limits".to_owned()))?;

        cpio::write_entry(&mut encoder, ino, &archive_path, entry.mode, size, |w| {
            copy_data(w, entry.reader, entry.len)
        })?;

        ino = ino
            .checked_add(1)
            .ok_or_else(|| RamuneError::CpioError("CPIO inode overflowed".to_owned()))?;
    }

    cpio::write_trailer(&mut encoder)?;
    encoder.finish().map_err(RamuneError::CompressionError)?;

    Ok(())
}

fn validate_entries(entries: &[Entry]) -> Result<()> {
    let mut prev: Option<&Path> = None;

    for entry in entries {
        validate_path_not_empty(entry)?;
        validate_not_absolute(entry)?;
        validate_no_parent_dir(entry)?;
        validate_no_duplicate(entry, prev)?;
        prev = Some(entry.archive_path);
    }

    Ok(())
}

fn validate_path_not_empty(entry: &Entry) -> Result<()> {
    if entry.archive_path.as_os_str().is_empty() {
        return Err(RamuneError::CpioError(
            "archive path must not be empty".to_owned(),
        ));
    }

    Ok(())
}

fn validate_not_absolute(entry: &Entry) -> Result<()> {
    if entry.archive_path.is_absolute() {
        return Err(RamuneError::CpioError(format!(
            "archive path must not be absolute: {}",
            entry.archive_path.display()
        )));
    }

    Ok(())
}

fn validate_no_parent_dir(entry: &Entry) -> Result<()> {
    if entry
        .archive_path
        .components()
        .any(|component| matches!(component, std::path::Component::ParentDir))
    {
        return Err(RamuneError::CpioError(format!(
            "archive path must not contain ..: {}",
            entry.archive_path.display()
        )));
    }

    Ok(())
}

fn validate_no_duplicate(entry: &Entry, prev: Option<&Path>) -> Result<()> {
    if let Some(previous) = prev
        && entry.archive_path == previous
    {
        return Err(RamuneError::CpioError(format!(
            "duplicate archive path: {}",
            entry.archive_path.display()
        )));
    }

    Ok(())
}

fn copy_data<W: Write>(writer: &mut W, reader: &mut dyn Read, len: u64) -> Result<()> {
    let mut limited = reader.take(len);
    let copied = std::io::copy(&mut limited, writer).map_err(|source| RamuneError::WriteError {
        file: String::new(),
        source,
    })?;

    if copied != len {
        return Err(RamuneError::CpioError(format!(
            "entry ended early: expected {len} bytes, copied {copied}"
        )));
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use core::str;
    use std::io::Cursor;

    use super::*;

    fn parse_newc_archive(bytes: &[u8]) -> Vec<(String, u32, Vec<u8>)> {
        let mut offset = 0_usize;
        let mut entries = Vec::new();

        loop {
            let header = bytes
                .get(offset..offset.saturating_add(110))
                .expect("header");
            let namesize = parse_hex(header.get(94..102).expect("namesize"));
            let filesize = parse_hex(header.get(54..62).expect("filesize"));
            let name_start = offset.saturating_add(110);
            let name_end = name_start.saturating_add(namesize);
            let name = str::from_utf8(
                bytes
                    .get(name_start..name_end.saturating_sub(1))
                    .expect("name bytes"),
            )
            .unwrap_or("")
            .to_owned();
            let data_start = name_end.next_multiple_of(4);
            let data_end = data_start.saturating_add(filesize);
            let data = bytes
                .get(data_start..data_end)
                .expect("data bytes")
                .to_vec();
            offset = data_end.next_multiple_of(4);

            match name.as_str() {
                "TRAILER!!!" => return entries,
                _ => entries.push((name, mode_from_header(header), data)),
            }
        }
    }

    fn parse_hex(field: &[u8]) -> usize {
        str::from_utf8(field)
            .ok()
            .and_then(|field| usize::from_str_radix(field, 16).ok())
            .unwrap_or(0)
    }

    fn mode_from_header(header: &[u8]) -> u32 {
        let field = header.get(14..22).expect("mode");
        str::from_utf8(field)
            .ok()
            .and_then(|field| u32::from_str_radix(field, 16).ok())
            .unwrap_or(0)
    }

    fn make_entry<'a>(
        archive_path: &'a Path,
        mode: u32,
        reader: &'a mut Cursor<Vec<u8>>,
    ) -> Entry<'a> {
        let len = reader.get_ref().len().try_into().unwrap_or(u64::MAX);
        Entry::new(archive_path, mode, reader, len)
    }

    #[test]
    fn empty_entries_returns_ok() {
        // ARRANGE
        let mut entries: [Entry<'_>; 0] = [];
        let mut buf = Vec::new();

        // ACT
        archive(&mut entries, 3, &mut buf).expect("archive should succeed");

        // ASSERT
        assert!(buf.is_empty());
    }

    #[test]
    fn writes_entries_in_sorted_order() {
        // ARRANGE
        let mut profile_reader = Cursor::new(b"profile".to_vec());
        let mut ext_reader = Cursor::new(b"extension".to_vec());
        let mut entries = [
            make_entry(Path::new("profile.toml"), 0o100_644, &mut profile_reader),
            make_entry(
                Path::new("extensions/test.erofs"),
                0o100_644,
                &mut ext_reader,
            ),
        ];

        // ACT
        let mut buf = Vec::new();
        archive(&mut entries, 3, &mut buf).expect("archive should succeed");

        // ASSERT
        let decoded = zstd::decode_all(buf.as_slice()).expect("decode");
        let parsed = parse_newc_archive(&decoded);
        assert_eq!(parsed.len(), 2);
        assert_eq!(
            parsed.first().map(|e| e.0.as_str()),
            Some("extensions/test.erofs")
        );
        assert_eq!(parsed.get(1).map(|e| e.0.as_str()), Some("profile.toml"));
    }

    #[test]
    fn rejects_empty_archive_path() {
        // ARRANGE
        let mut reader = Cursor::new(b"data".to_vec());
        let mut entries = [make_entry(Path::new(""), 0o100_644, &mut reader)];

        // ACT
        let mut buf = Vec::new();
        let result = archive(&mut entries, 3, &mut buf);

        // ASSERT
        assert!(result.is_err_and(|e| e.to_string().contains("must not be empty")));
    }

    #[test]
    fn rejects_absolute_archive_path() {
        // ARRANGE
        let mut reader = Cursor::new(b"data".to_vec());
        let mut entries = [make_entry(Path::new("/absolute"), 0o100_644, &mut reader)];

        // ACT
        let mut buf = Vec::new();
        let result = archive(&mut entries, 3, &mut buf);

        // ASSERT
        assert!(result.is_err_and(|e| e.to_string().contains("must not be absolute")));
    }

    #[test]
    fn rejects_parent_segments() {
        // ARRANGE
        let mut reader = Cursor::new(b"data".to_vec());
        let mut entries = [make_entry(Path::new("foo/../bar"), 0o100_644, &mut reader)];

        // ACT
        let mut buf = Vec::new();
        let result = archive(&mut entries, 3, &mut buf);

        // ASSERT
        assert!(result.is_err_and(|e| e.to_string().contains("must not contain ..")));
    }

    #[test]
    fn rejects_duplicate_paths() {
        // ARRANGE
        let mut first_reader = Cursor::new(b"first".to_vec());
        let mut second_reader = Cursor::new(b"second".to_vec());
        let mut entries = [
            make_entry(Path::new("dup"), 0o100_644, &mut first_reader),
            make_entry(Path::new("dup"), 0o100_644, &mut second_reader),
        ];

        // ACT
        let mut buf = Vec::new();
        let result = archive(&mut entries, 3, &mut buf);

        // ASSERT
        assert!(result.is_err_and(|e| e.to_string().contains("duplicate")));
    }

    #[test]
    fn rejects_short_reader() {
        // ARRANGE
        let mut reader = Cursor::new(b"data".to_vec());
        let mut entries = [Entry::new(
            Path::new("profile.toml"),
            0o100_644,
            &mut reader,
            32,
        )];

        // ACT
        let mut buf = Vec::new();
        let result = archive(&mut entries, 3, &mut buf);

        // ASSERT
        assert!(result.is_err_and(|e| e.to_string().contains("ended early")));
    }

    #[test]
    fn rejects_invalid_compression_level() {
        // ARRANGE
        let mut reader = Cursor::new(b"data".to_vec());
        let mut entries = [make_entry(
            Path::new("profile.toml"),
            0o100_644,
            &mut reader,
        )];

        // ACT
        let mut buf = Vec::new();
        let result = archive(&mut entries, i32::MAX, &mut buf);

        // ASSERT
        assert!(matches!(
            result,
            Err(RamuneError::InvalidCompressionLevel { .. })
        ));
    }
}
