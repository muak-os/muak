//! Creates a zstd-compressed or raw CPIO archive from a list of entries.

use std::io::{Read, Write};
use std::path::Path;

use crate::compress;
use crate::cpio;
use crate::error::{RamuneError, Result};

/// Core archive entry identity: destination path and payload length.
pub struct Entry<'a> {
    /// Destination path inside the archive.
    pub path: &'a Path,
    /// Exact payload length in bytes.
    pub len: u64,
}

/// An archive entry with a readable payload stream attached.
pub struct EntryStream<'a> {
    /// Core archive entry identity.
    pub entry: Entry<'a>,
    /// File mode (e.g. `0o100_755` for executables).
    pub mode: u32,
    /// Readable payload stream.
    pub reader: &'a mut dyn Read,
}

impl<'a> EntryStream<'a> {
    /// Creates an entry stream from a reader with an explicitly known payload length.
    pub fn new(path: &'a Path, mode: u32, reader: &'a mut dyn Read, len: u64) -> Self {
        EntryStream {
            entry: Entry { path, len },
            mode,
            reader,
        }
    }
}

/// Writes a zstd-compressed CPIO archive containing the given entries.
///
/// # Errors
///
/// Returns an error when validation fails, an entry exceeds CPIO limits,
/// reading an entry fails, or zstd compression fails.
pub fn compressed<W: Write>(
    streams: &mut [EntryStream],
    compression_level: i32,
    writer: &mut W,
) -> Result<()> {
    streams.sort_unstable_by(|left, right| left.entry.path.cmp(right.entry.path));
    validate_entries(streams)?;

    if streams.is_empty() {
        return Ok(());
    }

    let mut encoder = compress::encoder(writer, compression_level)?;
    let mut ino = 1_u32;

    for stream in streams.iter_mut() {
        let archive_path = stream.entry.path.to_string_lossy();
        let size = u32::try_from(stream.entry.len)
            .map_err(|_err| RamuneError::CpioError("file exceeds CPIO limits".to_owned()))?;

        cpio::write_entry(&mut encoder, ino, &archive_path, stream.mode, size, |w| {
            copy_data(w, stream.reader, stream.entry.len)
        })?;

        ino = ino
            .checked_add(1)
            .ok_or_else(|| RamuneError::CpioError("CPIO inode overflowed".to_owned()))?;
    }

    cpio::write_trailer(&mut encoder)?;
    encoder.finish().map_err(RamuneError::CompressionError)?;

    Ok(())
}

/// Writes an uncompressed CPIO `newc` archive containing the given entries.
///
/// # Errors
///
/// Returns an error when validation fails, an entry exceeds CPIO limits,
/// or reading/writing an entry fails.
pub fn raw<W: Write>(streams: &mut [EntryStream], writer: &mut W) -> Result<u64> {
    streams.sort_unstable_by(|left, right| left.entry.path.cmp(right.entry.path));
    validate_entries(streams)?;

    if streams.is_empty() {
        return Ok(0);
    }

    let mut count = CountWriter {
        inner: writer,
        written: 0,
    };
    let mut ino = 1_u32;

    for stream in streams.iter_mut() {
        let archive_path = stream.entry.path.to_string_lossy();
        let size = u32::try_from(stream.entry.len)
            .map_err(|_err| RamuneError::CpioError("file exceeds CPIO limits".to_owned()))?;

        cpio::write_entry(&mut count, ino, &archive_path, stream.mode, size, |w| {
            copy_data(w, stream.reader, stream.entry.len)
        })?;

        ino = ino
            .checked_add(1)
            .ok_or_else(|| RamuneError::CpioError("CPIO inode overflowed".to_owned()))?;
    }

    cpio::write_trailer(&mut count)?;
    Ok(count.written)
}

/// Returns the exact byte length of a raw CPIO `newc` archive.
#[must_use]
pub fn raw_size(entries: &[Entry]) -> u64 {
    if entries.is_empty() {
        return 0;
    }
    let mut sorted: Vec<_> = entries.iter().collect();
    sorted.sort_unstable_by(|left, right| left.path.cmp(right.path));
    sorted
        .iter()
        .map(|e| {
            let name = e.path.to_string_lossy();
            cpio::entry_size(&name, e.len)
        })
        .sum::<u64>()
        .saturating_add(cpio::trailer_size())
}

struct CountWriter<W> {
    inner: W,
    written: u64,
}

impl<W: Write> Write for CountWriter<W> {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        let n = self.inner.write(buf)?;
        let amount = u64::try_from(n).unwrap_or(u64::MAX);
        self.written = self.written.saturating_add(amount);

        Ok(n)
    }

    fn flush(&mut self) -> std::io::Result<()> {
        self.inner.flush()
    }
}

fn validate_entries(streams: &[EntryStream]) -> Result<()> {
    let mut prev: Option<&Path> = None;

    for stream in streams {
        validate_path_not_empty(stream)?;
        validate_not_absolute(stream)?;
        validate_no_parent_dir(stream)?;
        validate_no_duplicate(stream, prev)?;
        prev = Some(stream.entry.path);
    }

    Ok(())
}

fn validate_path_not_empty(stream: &EntryStream) -> Result<()> {
    if stream.entry.path.as_os_str().is_empty() {
        return Err(RamuneError::CpioError(
            "archive path must not be empty".to_owned(),
        ));
    }

    Ok(())
}

fn validate_not_absolute(stream: &EntryStream) -> Result<()> {
    if stream.entry.path.is_absolute() {
        return Err(RamuneError::CpioError(format!(
            "archive path must not be absolute: {}",
            stream.entry.path.display()
        )));
    }

    Ok(())
}

fn validate_no_parent_dir(stream: &EntryStream) -> Result<()> {
    if stream
        .entry
        .path
        .components()
        .any(|component| matches!(component, std::path::Component::ParentDir))
    {
        return Err(RamuneError::CpioError(format!(
            "archive path must not contain ..: {}",
            stream.entry.path.display()
        )));
    }

    Ok(())
}

fn validate_no_duplicate(stream: &EntryStream, prev: Option<&Path>) -> Result<()> {
    if let Some(previous) = prev
        && stream.entry.path == previous
    {
        return Err(RamuneError::CpioError(format!(
            "duplicate archive path: {}",
            stream.entry.path.display()
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
        path: &'a Path,
        mode: u32,
        reader: &'a mut Cursor<Vec<u8>>,
    ) -> EntryStream<'a> {
        let len = reader.get_ref().len().try_into().unwrap_or(u64::MAX);
        EntryStream::new(path, mode, reader, len)
    }

    #[test]
    fn empty_entries_returns_ok() {
        // ARRANGE
        let mut entries: [EntryStream<'_>; 0] = [];
        let mut buf = Vec::new();

        // ACT
        compressed(&mut entries, 3, &mut buf).expect("archive should succeed");

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
        compressed(&mut entries, 3, &mut buf).expect("archive should succeed");

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
        let result = compressed(&mut entries, 3, &mut buf);

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
        let result = compressed(&mut entries, 3, &mut buf);

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
        let result = compressed(&mut entries, 3, &mut buf);

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
        let result = compressed(&mut entries, 3, &mut buf);

        // ASSERT
        assert!(result.is_err_and(|e| e.to_string().contains("duplicate")));
    }

    #[test]
    fn rejects_short_reader() {
        // ARRANGE
        let mut reader = Cursor::new(b"data".to_vec());
        let mut entries = [EntryStream::new(
            Path::new("profile.toml"),
            0o100_644,
            &mut reader,
            32,
        )];

        // ACT
        let mut buf = Vec::new();
        let result = compressed(&mut entries, 3, &mut buf);

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
        let result = compressed(&mut entries, i32::MAX, &mut buf);

        // ASSERT
        assert!(matches!(
            result,
            Err(RamuneError::InvalidCompressionLevel { .. })
        ));
    }

    #[test]
    fn raw_writes_raw_cpio() {
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
        let written = raw(&mut entries, &mut buf).expect("raw should succeed");

        // ASSERT
        assert_eq!(written, u64::try_from(buf.len()).unwrap_or(0));
        assert!(!buf.is_empty());
        let parsed = parse_newc_archive(&buf);
        assert_eq!(parsed.len(), 2);
        assert_eq!(
            parsed.first().map(|e| e.0.as_str()),
            Some("extensions/test.erofs")
        );
        assert_eq!(parsed.get(1).map(|e| e.0.as_str()), Some("profile.toml"));
    }

    #[test]
    fn raw_size_matches_written_length() {
        // ARRANGE
        let profile_data = b"profile".to_vec();
        let ext_data = b"extension".to_vec();
        let profile_len = u64::try_from(profile_data.len()).unwrap_or(0);
        let ext_len = u64::try_from(ext_data.len()).unwrap_or(0);

        // ACT
        let size = raw_size(&[
            Entry {
                path: Path::new("profile.toml"),
                len: profile_len,
            },
            Entry {
                path: Path::new("extensions/test.erofs"),
                len: ext_len,
            },
        ]);

        let mut profile_reader = Cursor::new(profile_data);
        let mut ext_reader = Cursor::new(ext_data);
        let mut entries = [
            EntryStream::new(
                Path::new("profile.toml"),
                0o100_644,
                &mut profile_reader,
                profile_len,
            ),
            EntryStream::new(
                Path::new("extensions/test.erofs"),
                0o100_644,
                &mut ext_reader,
                ext_len,
            ),
        ];
        let mut buf = Vec::new();
        let written = raw(&mut entries, &mut buf).expect("raw should succeed");

        // ASSERT
        assert_eq!(size, written);
    }

    #[test]
    fn raw_empty_returns_zero() {
        // ARRANGE
        let mut entries: [EntryStream<'_>; 0] = [];
        let mut buf = Vec::new();

        // ACT
        let written = raw(&mut entries, &mut buf).expect("raw should succeed");

        // ASSERT
        assert_eq!(written, 0);
        assert!(buf.is_empty());
    }

    #[test]
    fn raw_size_empty_returns_zero() {
        // ARRANGE / ACT
        let size = raw_size(&[]);

        // ASSERT
        assert_eq!(size, 0);
    }

    #[test]
    fn raw_rejects_short_reader() {
        // ARRANGE
        let mut reader = Cursor::new(b"data".to_vec());
        let mut entries = [EntryStream::new(
            Path::new("profile.toml"),
            0o100_644,
            &mut reader,
            32,
        )];

        // ACT
        let mut buf = Vec::new();
        let result = raw(&mut entries, &mut buf);

        // ASSERT
        assert!(result.is_err_and(|e| e.to_string().contains("ended early")));
    }
}
