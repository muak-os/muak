//! Generic image payload packaging.

extern crate alloc;

use alloc::collections::BTreeSet;
use std::io::{self, Read, Write};
use std::path::Path;

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

/// A payload being assembled. File data is collected transiently during
/// [`plan`]; the buffers stay caller-owned and are dropped with the payload.
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
            MumiError::Io(io::Error::other(format!(
                "read payload file {}: {source}",
                entry.path
            )))
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

/// A planned, ready-to-write payload. Carries layout only; writing re-reads the
/// source payload's buffers through fresh `Read` views.
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
    pub fn write<W: Write>(&self, writer: &mut W, source: &Payload) -> Result<()> {
        let (_entries, mut readers) = entries_and_readers(&source.files, &source.buffers);
        let mut views = read_views(&mut readers);

        self.image.write(writer, &mut views)
    }
}

/// Plans one payload per source, measuring each into a layout-only [`Planned`].
///
/// # Errors
///
/// Returns an error when entries are invalid or an image cannot be planned.
pub fn plan(payloads: &mut [Payload], config: &image::BuildConfig) -> Result<Vec<Planned>> {
    let mut planned = Vec::with_capacity(payloads.len());

    for payload in payloads {
        let (entries, mut readers) = entries_and_readers(&payload.files, &payload.buffers);
        let mut views = read_views(&mut readers);
        let image = image::Image::build(&entries, &mut views, config)?;
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

/// The root, every intermediate parent directory, and the payload files in deterministic order.
fn entries_and_readers<'a>(
    files: &[FileEntry],
    buffers: &'a [Vec<u8>],
) -> (Vec<Entry>, Vec<SliceReader<'a>>) {
    let file_paths: BTreeSet<&str> = files.iter().map(|file| file.path.as_str()).collect();
    let mut dirs: BTreeSet<String> = BTreeSet::new();
    for file in files {
        let mut parent = parent_dir(&file.path);
        while parent != "/" {
            dirs.insert(parent.clone());
            parent = parent_dir(&parent);
        }
    }
    dirs.retain(|dir| !file_paths.contains(dir.as_str()));

    let mut entries = vec![Entry {
        path: "/".to_owned(),
        size: 0,
        mode: 0o040_755,
        symlink_target: Vec::new(),
    }];
    let mut readers = vec![SliceReader { data: &[] }];
    for dir in &dirs {
        entries.push(Entry {
            path: dir.clone(),
            size: 0,
            mode: 0o040_755,
            symlink_target: Vec::new(),
        });
        readers.push(SliceReader { data: &[] });
    }
    for (file, buffer) in files.iter().zip(buffers) {
        entries.push(Entry {
            path: file.path.clone(),
            size: file.size,
            mode: file.mode,
            symlink_target: Vec::new(),
        });
        readers.push(SliceReader { data: buffer });
    }

    (entries, readers)
}

/// The immediate parent of an absolute path, rooted at `/`.
fn parent_dir(path: &str) -> String {
    let parent = Path::new(path)
        .parent()
        .unwrap_or(Path::new("/"))
        .to_string_lossy()
        .into_owned();
    if parent.is_empty() {
        "/".to_owned()
    } else {
        parent
    }
}

/// Borrows a reader collection as plain `Read` views, one per element.
pub(crate) fn read_views<R: Read>(readers: &mut [R]) -> Vec<&mut dyn Read> {
    readers
        .iter_mut()
        .map(|reader| -> &mut dyn Read { reader })
        .collect()
}

/// A `Read` view over a byte slice, without allocation or seeking.
pub(crate) struct SliceReader<'a> {
    data: &'a [u8],
}

impl Read for SliceReader<'_> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let n = buf.len().min(self.data.len());
        let (head, tail) = self.data.split_at(n);
        let (dst, _) = buf.split_at_mut(n);
        dst.copy_from_slice(head);
        self.data = tail;

        Ok(n)
    }
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;

    use super::*;

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

    /// One independent reader per caller-owned buffer.
    fn buffer_readers(buffers: &[Vec<u8>]) -> Vec<SliceReader<'_>> {
        buffers
            .iter()
            .map(|buffer| SliceReader { data: buffer })
            .collect()
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
        let source = payloads.first().expect("p");
        let mut buf = Vec::new();
        payload.write(&mut buf, source).expect("write payload");

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
    fn nested_files_plan_with_directories() {
        // ARRANGE
        let mut payload = Payload::new("p");
        let mut state = 0xdead_beef_u64;
        let mut data = Vec::with_capacity(8192);
        for _ in 0..8192 {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            data.push(u8::try_from(state & 0xff).unwrap_or(0));
        }
        let (entry, mut data) = file("/usr/bin/tool", data.len(), &data);
        payload.add_file(entry, &mut data).expect("add file");
        let mut payloads = [payload];

        // ACT
        let planned = plan(&mut payloads, &config()).expect("plan payloads");
        let payload = planned.first().expect("one payload");

        // ASSERT
        assert!(
            payload.size() > 4096,
            "nested payload must carry the file data, not just the root directory"
        );
    }

    #[test]
    fn nested_files_round_trip_size() {
        // ARRANGE
        let mut payload = Payload::new("p");
        let (entry, mut data) = file("/lib/modules/7.2.0-muak/kernel/foo.ko.zst", 5, b"kdata");
        payload.add_file(entry, &mut data).expect("add file");
        let mut payloads = [payload];

        // ACT
        let planned = plan(&mut payloads, &config()).expect("plan payloads");
        let payload = planned.first().expect("one payload");
        let source = payloads.first().expect("source");
        let mut buf = Vec::new();
        payload.write(&mut buf, source).expect("write payload");

        // ASSERT
        assert_eq!(u64::try_from(buf.len()).unwrap_or(0), payload.size());
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
    fn read_views_exhaust_buffers() {
        // ARRANGE
        let buffers = vec![b"ab".to_vec()];
        let mut readers = buffer_readers(&buffers);
        let mut views = read_views(&mut readers);
        let mut buf = [0_u8; 4];

        // ACT
        let view: &mut dyn Read = &mut **views.get_mut(0).expect("one view");
        let first = view.read(&mut buf).expect("read");
        let second = view.read(&mut buf).expect("read");

        // ASSERT
        assert_eq!(first, 2);
        assert_eq!(second, 0);
    }

    #[test]
    fn plan_is_deterministic_over_identical_content() {
        // ARRANGE
        let content = vec![0xAB_u8; 16_384];
        let build = |data: Vec<u8>| {
            let (entry, mut reader) = file("/f", data.len(), &data);
            let mut payload = Payload::new("p");
            payload.add_file(entry, &mut reader).expect("add file");
            plan(&mut [payload], &config()).expect("plan")
        };

        // ACT
        let first = build(content.clone());
        let second = build(content.clone());

        // ASSERT
        let size1 = first.first().expect("one").size();
        let size2 = second.first().expect("one").size();
        assert_eq!(size1, size2, "identical content must measure identically");
    }

    #[test]
    fn plan_size_tracks_content_changes() {
        // ARRANGE
        let small = vec![0xAB_u8; 16_384];
        let large = vec![0xAB_u8; 32_768];
        let plan_for = |data: Vec<u8>| {
            let (entry, mut reader) = file("/f", data.len(), &data);
            let mut payload = Payload::new("p");
            payload.add_file(entry, &mut reader).expect("add file");
            plan(&mut [payload], &config()).expect("plan")
        };

        // ACT
        let planned_small = plan_for(small);
        let planned_large = plan_for(large);

        // ASSERT
        assert_ne!(
            planned_small.first().expect("one").size(),
            planned_large.first().expect("one").size(),
            "different content must measure to different payload sizes"
        );
    }
}
