//! Generic image payload packaging.
//! TODO: Remove buffering

use std::io::{self, Read, Write};

use crate::error::{MumiError, Result};
use crate::image::{self, Entry};

/// Format identifier for payload images, owned by mumi.
const FORMAT: &str = ".erofs";

/// Metadata describing a planned payload. All fields are public.
pub struct Meta {
    /// Opaque caller-supplied identity, stored unchanged.
    pub name: String,
    /// Format identifier.
    pub format: String,
    /// Exact payload byte count, known before any payload bytes stream.
    pub size: u64,
}

/// A file entry feeding a payload image.
pub struct FileEntry {
    /// Path inside the image (absolute, e.g. `/usr/bin/tool`).
    pub path: String,
    /// File size in bytes.
    pub size: u64,
    /// File mode including file-type bits (e.g. `0o100_755`).
    pub mode: u32,
}

/// A payload being assembled. File data is streamed in single-pass and
/// buffered internally by mumi.
pub struct Payload {
    name: String,
    files: Vec<FileEntry>,
    buffers: Vec<Vec<u8>>,
}

impl Payload {
    /// Creates a payload with the given identity.
    #[must_use]
    pub fn new<S: Into<String>>(name: S) -> Self {
        Self {
            name: name.into(),
            files: Vec::new(),
            buffers: Vec::new(),
        }
    }

    /// Streams one file's bytes single-pass into the internal buffer.
    ///
    /// # Errors
    ///
    /// Returns an error when fewer bytes than `entry.size` are read.
    pub fn add_file(&mut self, entry: FileEntry, data: &mut dyn Read) -> Result<()> {
        let mut buf = Vec::with_capacity(usize::try_from(entry.size).unwrap_or(usize::MAX));
        data.read_to_end(&mut buf).map_err(|source| {
            MumiError::Io(io::Error::new(
                source.kind(),
                format!("read payload file {}: {source}", entry.path),
            ))
        })?;
        if u64::try_from(buf.len()).unwrap_or(u64::MAX) != entry.size {
            return Err(MumiError::InvalidArgument(format!(
                "payload file {} size mismatch: expected {}, got {}",
                entry.path,
                entry.size,
                buf.len(),
            )));
        }
        self.files.push(entry);
        self.buffers.push(buf);

        Ok(())
    }
}

/// A planned, ready-to-write payload. All fields are private.
pub struct Planned {
    meta: Meta,
    image: image::Image,
}

impl Planned {
    /// Returns the payload metadata.
    #[must_use]
    pub fn meta(&self) -> &Meta {
        &self.meta
    }

    /// Returns the exact payload byte count.
    #[must_use]
    pub fn size(&self) -> u64 {
        self.meta.size
    }

    /// Writes the complete payload, emitting exactly `size` bytes.
    ///
    /// # Errors
    ///
    /// Returns an error when image serialization or data emission fails.
    pub fn write<W: Write>(&self, writer: &mut W) -> Result<()> {
        self.image.write(writer)
    }
}

/// Plans one payload per source. Reads and buffers all file data internally.
///
/// # Errors
///
/// Returns an error when entries are invalid or an image cannot be planned.
pub fn plan(payloads: &mut [Payload], config: &image::BuildConfig) -> Result<Vec<Planned>> {
    let mut planned = Vec::with_capacity(payloads.len());

    for payload in payloads {
        let entries: Vec<Entry> = payload.files.iter().map(to_image_entry).collect();
        let mut reader = BufferReader {
            buffers: &payload.buffers,
            positions: vec![0; payload.buffers.len()],
        };
        let image = image::build(&payload.name, &entries, &mut reader, config)?;
        planned.push(Planned {
            meta: Meta {
                name: payload.name.clone(),
                format: FORMAT.to_owned(),
                size: image.len(),
            },
            image,
        });
    }

    Ok(planned)
}

/// A positional `Reader` over an owned buffer set.
struct BufferReader<'a> {
    buffers: &'a [Vec<u8>],
    positions: Vec<usize>,
}

impl image::Reader for BufferReader<'_> {
    fn read(&mut self, index: usize, buf: &mut [u8]) -> io::Result<usize> {
        let file = self
            .buffers
            .get(index)
            .ok_or_else(|| io::Error::other("file out of bounds"))?;
        let position = self
            .positions
            .get_mut(index)
            .ok_or_else(|| io::Error::other("position out of bounds"))?;
        let remaining = file.len().saturating_sub(*position);
        let n = remaining.min(buf.len());
        let data = file
            .get(*position..position.saturating_add(n))
            .unwrap_or_default();
        buf.get_mut(..n)
            .ok_or_else(|| io::Error::other("buffer too small"))?
            .copy_from_slice(data);
        *position = position.saturating_add(n);

        Ok(n)
    }
}

/// Maps a caller file entry to an image entry.
fn to_image_entry(file: &FileEntry) -> Entry {
    Entry {
        path: file.path.clone(),
        size: file.size,
        mode: file.mode,
        symlink_target: Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;

    use super::*;
    use crate::image::Reader as _;

    fn file(path: &str, size: usize, data: &[u8]) -> (FileEntry, Cursor<Vec<u8>>) {
        (
            FileEntry {
                path: path.to_owned(),
                size: u64::try_from(size).unwrap_or(0),
                mode: 0o100_644,
            },
            Cursor::new(data.to_vec()),
        )
    }

    fn config() -> image::BuildConfig {
        image::BuildConfig {
            compression_level: crate::DEFAULT_ZSTD_COMPRESSION_LEVEL,
            file_contexts: None,
        }
    }

    #[test]
    fn plans_one_payload_per_source_in_order() {
        // ARRANGE
        let (entry_a, mut data_a) = file("/usr/bin/a", 1, b"x");
        let (entry_b, mut data_b) = file("/usr/bin/b", 1, b"y");
        let mut payload_a = Payload::new("first");
        payload_a.add_file(entry_a, &mut data_a).expect("add file");
        let mut payload_b = Payload::new("second");
        payload_b.add_file(entry_b, &mut data_b).expect("add file");
        let mut payloads = [payload_a, payload_b];

        // ACT
        let planned = plan(&mut payloads, &config()).expect("plan payloads");

        // ASSERT
        assert_eq!(planned.len(), 2);
        assert_eq!(planned.first().expect("first").meta().name, "first");
        assert_eq!(planned.get(1).expect("second").meta().name, "second");
    }

    #[test]
    fn meta_format_is_mumi_owned() {
        // ARRANGE
        let (entry, mut data) = file("/f", 2, b"ab");
        let mut payload = Payload::new("muak-os/qemu");
        payload.add_file(entry, &mut data).expect("add file");
        let mut payloads = [payload];

        // ACT
        let planned = plan(&mut payloads, &config()).expect("plan payloads");

        // ASSERT
        assert_eq!(planned.first().expect("one").meta().format, FORMAT);
    }

    #[test]
    fn meta_name_is_forwarded_unchanged() {
        // ARRANGE
        let (entry, mut data) = file("/f", 2, b"ab");
        let mut payload = Payload::new("muak-os/qemu");
        payload.add_file(entry, &mut data).expect("add file");
        let mut payloads = [payload];

        // ACT
        let planned = plan(&mut payloads, &config()).expect("plan payloads");

        // ASSERT
        assert_eq!(planned.first().expect("one").meta().name, "muak-os/qemu");
    }

    #[test]
    fn size_matches_written_bytes() {
        // ARRANGE
        let (entry, mut data) = file("/f", 8, b"data....");
        let mut payload = Payload::new("p");
        payload.add_file(entry, &mut data).expect("add file");
        let mut payloads = [payload];

        // ACT
        let planned = plan(&mut payloads, &config()).expect("plan payloads");
        let payload = planned.first().expect("one payload");
        let mut buf = Vec::new();
        payload.write(&mut buf).expect("write payload");

        // ASSERT
        assert_eq!(payload.size(), payload.meta().size);
        assert!(!buf.is_empty());
        assert_eq!(u64::try_from(buf.len()).unwrap_or(0), payload.size());
    }

    #[test]
    fn empty_payloads_plans_empty() {
        // ARRANGE
        let mut payloads: [Payload; 0] = [];

        // ACT
        let planned = plan(&mut payloads, &config()).expect("plan payloads");

        // ASSERT
        assert!(planned.is_empty());
    }

    #[test]
    fn add_file_rejects_short_data() {
        // ARRANGE
        let (entry, mut data) = file("/f", 4, b"ab");
        let mut payload = Payload::new("p");

        // ACT
        let result = payload.add_file(entry, &mut data);

        // ASSERT
        assert!(result.is_err());
    }

    #[test]
    fn reader_exhausted_returns_zero() {
        // ARRANGE
        let mut reader = BufferReader {
            buffers: &[b"ab".to_vec()],
            positions: vec![0],
        };
        let mut buf = [0_u8; 4];
        reader.read(0, &mut buf).expect("read");

        // ACT
        let n = reader.read(0, &mut buf).expect("read");

        // ASSERT
        assert_eq!(n, 0);
    }
}
