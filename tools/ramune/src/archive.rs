//! CPIO archive writing with optional zstd compression.

use std::io::Write;
use std::path::Path;

use crate::compress;
use crate::cpio;
use crate::error::{RamuneError, Result};

/// An entry in a CPIO archive.
pub struct Entry {
    /// Destination path inside the archive.
    pub path: String,
    /// File mode (e.g. `0o100_755` for executables).
    pub mode: u32,
    /// Exact payload length in bytes.
    pub len: u64,
}

/// Writes a raw CPIO archive. Entries are sorted by path.
///
/// # Errors
///
/// Returns an error when validation fails, an entry exceeds CPIO limits,
/// or the data callback fails.
pub fn cpio<W: Write, F: FnMut(&Entry, &mut W) -> Result<()>>(
    entries: &mut [Entry],
    writer: &mut W,
    mut data: F,
) -> Result<u64> {
    entries.sort_unstable_by(|left, right| left.path.cmp(&right.path));
    validate_entries(entries)?;

    if entries.is_empty() {
        return Ok(0);
    }

    let total = entries
        .iter()
        .map(|e| cpio::entry_size(&e.path, e.len))
        .sum::<u64>()
        .saturating_add(cpio::trailer_size());

    let mut ino = 1_u32;

    for entry in entries.iter() {
        let size = u32::try_from(entry.len)
            .map_err(|_err| RamuneError::CpioError("file exceeds CPIO limits".to_owned()))?;

        cpio::write_entry(writer, ino, &entry.path, entry.mode, size, |w| {
            data(entry, w)
        })?;

        ino = ino
            .checked_add(1)
            .ok_or_else(|| RamuneError::CpioError("CPIO inode overflowed".to_owned()))?;
    }

    cpio::write_trailer(writer)?;

    Ok(total)
}

/// Writes a zstd-compressed CPIO archive. Wraps `writer` in a zstd
/// encoder, calls [`cpio`], then finishes the encoder.
///
/// # Errors
///
/// Returns an error when validation fails, an entry exceeds CPIO limits,
/// the data callback fails or zstd compression fails.
pub fn compressed<W: Write, F: FnMut(&Entry, &mut zstd::Encoder<'static, &mut W>) -> Result<()>>(
    entries: &mut [Entry],
    writer: &mut W,
    compression_level: i32,
    mut data: F,
) -> Result<()> {
    let mut encoder = compress::encoder(writer, compression_level)?;
    cpio(entries, &mut encoder, &mut data)?;
    encoder.finish().map_err(RamuneError::CompressionError)?;

    Ok(())
}

/// Returns the exact byte length of a raw CPIO `newc` archive.
#[must_use]
pub fn size(entries: &[Entry]) -> u64 {
    if entries.is_empty() {
        return 0;
    }
    let mut order: Vec<usize> = (0..entries.len()).collect();
    order.sort_unstable_by(|&left, &right| {
        entries
            .get(left)
            .zip(entries.get(right))
            .map_or(core::cmp::Ordering::Equal, |(left_entry, right_entry)| {
                left_entry.path.cmp(&right_entry.path)
            })
    });

    order
        .iter()
        .filter_map(|&i| entries.get(i))
        .map(|e| cpio::entry_size(&e.path, e.len))
        .sum::<u64>()
        .saturating_add(cpio::trailer_size())
}

fn validate_entries(entries: &[Entry]) -> Result<()> {
    let mut prev: Option<&str> = None;

    for entry in entries {
        validate_path_not_empty(entry)?;
        validate_not_absolute(entry)?;
        validate_no_parent_dir(entry)?;
        validate_no_duplicate(entry, prev)?;
        prev = Some(&entry.path);
    }

    Ok(())
}

fn validate_path_not_empty(entry: &Entry) -> Result<()> {
    if entry.path.is_empty() {
        return Err(RamuneError::CpioError(
            "archive path must not be empty".to_owned(),
        ));
    }

    Ok(())
}

fn validate_not_absolute(entry: &Entry) -> Result<()> {
    if Path::new(&entry.path).is_absolute() {
        return Err(RamuneError::CpioError(format!(
            "archive path must not be absolute: {}",
            entry.path
        )));
    }

    Ok(())
}

fn validate_no_parent_dir(entry: &Entry) -> Result<()> {
    if Path::new(&entry.path)
        .components()
        .any(|component| matches!(component, std::path::Component::ParentDir))
    {
        return Err(RamuneError::CpioError(format!(
            "archive path must not contain ..: {}",
            entry.path
        )));
    }

    Ok(())
}

fn validate_no_duplicate(entry: &Entry, prev: Option<&str>) -> Result<()> {
    if let Some(previous) = prev
        && entry.path == previous
    {
        return Err(RamuneError::CpioError(format!(
            "duplicate archive path: {}",
            entry.path
        )));
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use std::io::Read as _;

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
            let name = core::str::from_utf8(
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
        core::str::from_utf8(field)
            .ok()
            .and_then(|field| usize::from_str_radix(field, 16).ok())
            .unwrap_or(0)
    }

    fn mode_from_header(header: &[u8]) -> u32 {
        let field = header.get(14..22).expect("mode");
        core::str::from_utf8(field)
            .ok()
            .and_then(|field| u32::from_str_radix(field, 16).ok())
            .unwrap_or(0)
    }

    #[test]
    fn empty_entries_returns_zero() {
        // ARRANGE
        let mut entries: [Entry; 0] = [];
        let mut buf = Vec::new();

        // ACT
        let written = cpio(&mut entries, &mut buf, |_, _| Ok(())).expect("write");

        // ASSERT
        assert_eq!(written, 0);
        assert!(buf.is_empty());
    }

    #[test]
    fn writes_entries_in_sorted_order() {
        // ARRANGE
        let mut entries = [
            Entry {
                path: "profile.toml".into(),
                mode: 0o100_644,
                len: 7,
            },
            Entry {
                path: "extensions/test.erofs".into(),
                mode: 0o100_644,
                len: 9,
            },
        ];

        // ACT
        let mut buf = Vec::new();
        cpio(&mut entries, &mut buf, |entry, w| {
            let data: &[u8] = match entry.path.as_str() {
                "profile.toml" => b"profile",
                "extensions/test.erofs" => b"extension",
                other => panic!("unknown entry: {other}"),
            };
            let mut limited = std::io::Cursor::new(data).take(entry.len);
            std::io::copy(&mut limited, w).map_err(|e| RamuneError::WriteError {
                file: String::new(),
                source: e,
            })?;
            Ok(())
        })
        .expect("write_cpio");

        // ASSERT
        let parsed = parse_newc_archive(&buf);
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
        let mut entries = [Entry {
            path: String::new(),
            mode: 0o100_644,
            len: 4,
        }];
        let mut buf = Vec::new();

        // ACT
        let result = cpio(&mut entries, &mut buf, |_, _| Ok(()));

        // ASSERT
        assert!(result.is_err_and(|e| e.to_string().contains("must not be empty")));
    }

    #[test]
    fn rejects_absolute_archive_path() {
        // ARRANGE
        let mut entries = [Entry {
            path: "/absolute".into(),
            mode: 0o100_644,
            len: 4,
        }];
        let mut buf = Vec::new();

        // ACT
        let result = cpio(&mut entries, &mut buf, |_, _| Ok(()));

        // ASSERT
        assert!(result.is_err_and(|e| e.to_string().contains("must not be absolute")));
    }

    #[test]
    fn rejects_parent_segments() {
        // ARRANGE
        let mut entries = [Entry {
            path: "foo/../bar".into(),
            mode: 0o100_644,
            len: 4,
        }];
        let mut buf = Vec::new();

        // ACT
        let result = cpio(&mut entries, &mut buf, |_, _| Ok(()));

        // ASSERT
        assert!(result.is_err_and(|e| e.to_string().contains("must not contain ..")));
    }

    #[test]
    fn rejects_duplicate_paths() {
        // ARRANGE
        let mut entries = [
            Entry {
                path: "dup".into(),
                mode: 0o100_644,
                len: 5,
            },
            Entry {
                path: "dup".into(),
                mode: 0o100_644,
                len: 6,
            },
        ];
        let mut buf = Vec::new();

        // ACT
        let result = cpio(&mut entries, &mut buf, |_, _| Ok(()));

        // ASSERT
        assert!(result.is_err_and(|e| e.to_string().contains("duplicate")));
    }

    #[test]
    fn write_cpio_returns_correct_size() {
        // ARRANGE
        let mut entries = [Entry {
            path: "a".into(),
            mode: 0o100_644,
            len: 5,
        }];
        let mut buf = Vec::new();

        // ACT
        let written = cpio(&mut entries, &mut buf, |_, w| {
            w.write_all(b"hello").map_err(|e| RamuneError::WriteError {
                file: String::new(),
                source: e,
            })
        })
        .expect("write_cpio");

        // ASSERT
        assert_eq!(u64::try_from(buf.len()).unwrap_or(0), written);
        assert_eq!(written, size(&entries));
    }

    #[test]
    fn raw_size_empty_returns_zero() {
        // ARRANGE / ACT
        let size = size(&[]);

        // ASSERT
        assert_eq!(size, 0);
    }

    #[test]
    fn compressed_produces_valid_archive() {
        // ARRANGE
        let mut entries = [Entry {
            path: "a".into(),
            mode: 0o100_644,
            len: 5,
        }];
        let mut buf = Vec::new();

        // ACT
        compressed(&mut entries, &mut buf, 3, |_, w| {
            w.write_all(b"hello").map_err(|e| RamuneError::WriteError {
                file: String::new(),
                source: e,
            })
        })
        .expect("compressed");

        // ASSERT
        let decoded = zstd::decode_all(buf.as_slice()).expect("decode");
        let parsed = parse_newc_archive(&decoded);
        assert_eq!(parsed.len(), 1);
        assert_eq!(parsed.first().map(|e| &e.2), Some(&b"hello".to_vec()));
    }
}
