//! Initramfs tail generation by writing a compressed archive of append entries.

use std::io::{Read, Write};
use std::path::Path;

use crate::compress;
use crate::cpio;
use crate::error::{RamuneError, Result};

/// Settings for building a compressed append tail.
pub struct TailConfig<'a> {
    /// Entries to include in the appended archive.
    pub entries: &'a mut [AppendEntry<'a>],
    /// Zstd compression level for the appended archive.
    pub compression_level: i32,
}

/// A single finalized entry to append to the initramfs tail.
pub struct AppendEntry<'a> {
    /// Destination path inside the appended CPIO archive.
    pub archive_path: &'a Path,
    /// File mode to encode in the CPIO entry.
    pub mode: u32,
    /// Exact payload length in bytes.
    pub len: u64,
    /// Readable payload stream.
    pub reader: &'a mut dyn Read,
}

/// Builds a zstd-compressed CPIO archive containing the configured entries.
///
/// Entries are sorted by `archive_path` in-place before compression.
///
/// # Errors
///
/// Returns an error when validation fails, entry lengths exceed CPIO limits,
/// reading an entry fails, or zstd compression fails.
pub fn build_tail(config: &mut TailConfig<'_>) -> Result<Vec<u8>> {
    config
        .entries
        .sort_unstable_by(|left, right| left.archive_path.cmp(right.archive_path));
    validate_entries(config.entries)?;

    if config.entries.is_empty() {
        return Ok(Vec::new());
    }

    write_compressed_cpio_archive(config.entries, config.compression_level)
}

fn validate_entries(entries: &[AppendEntry<'_>]) -> Result<()> {
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

fn validate_path_not_empty(entry: &AppendEntry<'_>) -> Result<()> {
    if entry.archive_path.as_os_str().is_empty() {
        return Err(RamuneError::CpioError(
            "archive path must not be empty".to_owned(),
        ));
    }

    Ok(())
}

fn validate_not_absolute(entry: &AppendEntry<'_>) -> Result<()> {
    if entry.archive_path.is_absolute() {
        return Err(RamuneError::CpioError(format!(
            "archive path must not be absolute: {}",
            entry.archive_path.display()
        )));
    }

    Ok(())
}

fn validate_no_parent_dir(entry: &AppendEntry<'_>) -> Result<()> {
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

fn validate_no_duplicate(entry: &AppendEntry<'_>, prev: Option<&Path>) -> Result<()> {
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

fn write_compressed_cpio_archive(
    entries: &mut [AppendEntry<'_>],
    compression_level: i32,
) -> Result<Vec<u8>> {
    let mut encoder = compress::encoder(Vec::new(), compression_level)?;
    let mut ino = 1_u32;

    for entry in entries {
        let archive_path = entry.archive_path.to_string_lossy();
        let size = u32::try_from(entry.len)
            .map_err(|_err| RamuneError::CpioError("extra file exceeds CPIO limits".to_owned()))?;

        cpio::write_entry(&mut encoder, ino, &archive_path, entry.mode, size, |w| {
            copy_entry_data(w, entry.reader, entry.len)
        })?;

        ino = ino
            .checked_add(1)
            .ok_or_else(|| RamuneError::CpioError("CPIO inode overflowed".to_owned()))?;
    }

    cpio::write_trailer(&mut encoder)?;

    encoder.finish().map_err(RamuneError::CompressionError)
}

fn copy_entry_data<W: Write>(writer: &mut W, reader: &mut dyn Read, len: u64) -> Result<()> {
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
    use std::path::Path;

    use super::*;

    fn entry<'a>(
        archive_path: &'a Path,
        mode: u32,
        reader: &'a mut Cursor<Vec<u8>>,
    ) -> AppendEntry<'a> {
        let len = u64::try_from(reader.get_ref().len()).unwrap_or(0);
        AppendEntry {
            archive_path,
            mode,
            len,
            reader,
        }
    }

    fn decode_tail(tail: &[u8]) -> Vec<(String, Vec<u8>)> {
        let archive = zstd::decode_all(tail).expect("decode tail");
        let mut offset = 0_usize;
        let mut entries = Vec::new();

        loop {
            let header = archive
                .get(offset..offset.saturating_add(110))
                .expect("header");
            let namesize = parse_hex(header.get(94..102).expect("namesize"));
            let filesize = parse_hex(header.get(54..62).expect("filesize"));
            let name_start = offset.saturating_add(110);
            let name_end = name_start.saturating_add(namesize);
            let name = str::from_utf8(
                archive
                    .get(name_start..name_end.saturating_sub(1))
                    .expect("name bytes"),
            )
            .unwrap_or("")
            .to_owned();
            let data_start = name_end.next_multiple_of(4);
            let data_end = data_start.saturating_add(filesize);
            let data = archive
                .get(data_start..data_end)
                .expect("data bytes")
                .to_vec();
            offset = data_end.next_multiple_of(4);

            match name.as_str() {
                "TRAILER!!!" => return entries,
                _ => entries.push((name, data)),
            }
        }
    }

    fn parse_hex(field: &[u8]) -> usize {
        str::from_utf8(field)
            .ok()
            .and_then(|field| usize::from_str_radix(field, 16).ok())
            .unwrap_or(0)
    }

    #[test]
    fn build_tail_returns_empty_vec_for_no_entries() {
        // ARRANGE
        let mut entries: [AppendEntry<'_>; 0] = [];

        // ACT
        let tail = build_tail(&mut TailConfig {
            entries: &mut entries,
            compression_level: 3,
        })
        .expect("build tail");

        // ASSERT
        assert!(tail.is_empty());
    }

    #[test]
    fn build_tail_writes_entries_in_sorted_order() {
        // ARRANGE
        let mut profile_reader = Cursor::new(b"profile".to_vec());
        let mut ext_reader = Cursor::new(b"extension".to_vec());
        let mut entries = [
            entry(Path::new("profile.toml"), 0o100_644, &mut profile_reader),
            entry(
                Path::new("extensions/test.erofs"),
                0o100_644,
                &mut ext_reader,
            ),
        ];

        // ACT
        let tail = build_tail(&mut TailConfig {
            entries: &mut entries,
            compression_level: 3,
        })
        .expect("build tail");

        // ASSERT
        let decoded = decode_tail(&tail);
        assert_eq!(decoded.len(), 2);
        assert_eq!(
            decoded.first().map(|entry| entry.0.as_str()),
            Some("extensions/test.erofs")
        );
        assert_eq!(
            decoded.get(1).map(|entry| entry.0.as_str()),
            Some("profile.toml")
        );
    }

    #[test]
    fn build_tail_rejects_empty_archive_path() {
        // ARRANGE
        let mut reader = Cursor::new(b"data".to_vec());
        let mut entries = [entry(Path::new(""), 0o100_644, &mut reader)];

        // ACT
        let result = build_tail(&mut TailConfig {
            entries: &mut entries,
            compression_level: 3,
        });

        // ASSERT
        assert!(result.is_err_and(|error| error.to_string().contains("must not be empty")));
    }

    #[test]
    fn build_tail_rejects_absolute_archive_path() {
        // ARRANGE
        let mut reader = Cursor::new(b"data".to_vec());
        let mut entries = [entry(Path::new("/absolute"), 0o100_644, &mut reader)];

        // ACT
        let result = build_tail(&mut TailConfig {
            entries: &mut entries,
            compression_level: 3,
        });

        // ASSERT
        assert!(result.is_err_and(|error| error.to_string().contains("must not be absolute")));
    }

    #[test]
    fn build_tail_rejects_parent_segments() {
        // ARRANGE
        let mut reader = Cursor::new(b"data".to_vec());
        let mut entries = [entry(Path::new("foo/../bar"), 0o100_644, &mut reader)];

        // ACT
        let result = build_tail(&mut TailConfig {
            entries: &mut entries,
            compression_level: 3,
        });

        // ASSERT
        assert!(result.is_err_and(|error| error.to_string().contains("must not contain ..")));
    }

    #[test]
    fn build_tail_rejects_duplicate_paths() {
        // ARRANGE
        let mut first_reader = Cursor::new(b"first".to_vec());
        let mut second_reader = Cursor::new(b"second".to_vec());
        let mut entries = [
            entry(Path::new("dup"), 0o100_644, &mut first_reader),
            entry(Path::new("dup"), 0o100_644, &mut second_reader),
        ];

        // ACT
        let result = build_tail(&mut TailConfig {
            entries: &mut entries,
            compression_level: 3,
        });

        // ASSERT
        assert!(result.is_err_and(|error| error.to_string().contains("duplicate")));
    }

    #[test]
    fn build_tail_rejects_short_reader() {
        // ARRANGE
        let mut reader = Cursor::new(b"data".to_vec());
        let mut entries = [AppendEntry {
            archive_path: Path::new("profile.toml"),
            mode: 0o100_644,
            len: 32,
            reader: &mut reader,
        }];

        // ACT
        let result = build_tail(&mut TailConfig {
            entries: &mut entries,
            compression_level: 3,
        });

        // ASSERT
        assert!(result.is_err_and(|error| error.to_string().contains("ended early")));
    }

    #[test]
    fn build_tail_rejects_invalid_compression_level() {
        // ARRANGE
        let mut reader = Cursor::new(b"data".to_vec());
        let mut entries = [entry(Path::new("profile.toml"), 0o100_644, &mut reader)];

        // ACT
        let result = build_tail(&mut TailConfig {
            entries: &mut entries,
            compression_level: i32::MAX,
        });

        // ASSERT
        assert!(matches!(
            result,
            Err(RamuneError::InvalidCompressionLevel { .. })
        ));
    }
}
