//! One-pass routing of streamed tar entries to named writers.

use std::collections::HashMap;
use std::io::{Read, Write};

use super::entries::FileEntry;
use crate::error::{KociError, Result};

/// Routes streamed tar entries to their writers in a single pass over the image.
pub struct Demux<W: Write> {
    targets: HashMap<String, Target<W>>,
}

struct Target<W> {
    writer: W,
    expected: Option<u64>,
}

impl<W: Write> Demux<W> {
    /// Build a router from expected entry names to their writers.
    #[must_use]
    pub fn by_name(map: HashMap<String, W>) -> Self {
        Self {
            targets: map
                .into_iter()
                .map(|(name, writer)| {
                    (
                        name,
                        Target {
                            writer,
                            expected: None,
                        },
                    )
                })
                .collect(),
        }
    }

    /// Record the expected size of each routed entry, for fail-fast size guards.
    #[must_use]
    pub fn with_sizes<I: IntoIterator<Item = (String, u64)>>(mut self, sizes: I) -> Self {
        for (name, expected) in sizes {
            self.set_expected(&name, expected);
        }

        self
    }

    /// Route one streamed entry: copy it to its writer, or drain it when unknown.
    ///
    /// # Errors
    ///
    /// Returns an error when a size guard fails or a copy fails.
    pub fn route(&mut self, entry: FileEntry<'_>) -> Result<()> {
        let FileEntry {
            path, size, reader, ..
        } = entry;
        let Some(target) = self.targets.get_mut(&path) else {
            return drain(reader);
        };

        copy_target(target, &path, reader, size)
    }

    fn set_expected(&mut self, name: &str, expected: u64) {
        if let Some(target) = self.targets.get_mut(name) {
            target.expected = Some(expected);
        }
    }

    /// Consume the router, returning its writers keyed by entry name.
    #[must_use]
    pub fn into_writers(self) -> HashMap<String, W> {
        self.targets
            .into_iter()
            .map(|(name, target)| (name, target.writer))
            .collect()
    }
}

fn copy_target<W: Write>(
    target: &mut Target<W>,
    name: &str,
    reader: &mut dyn Read,
    size: u64,
) -> Result<()> {
    if let Some(expected) = target.expected.filter(|expected| *expected != size) {
        return Err(KociError::Pull(format!(
            "demux size mismatch for {name}: annotated {expected}, tar says {size}"
        )));
    }

    let copied = std::io::copy(reader, &mut target.writer)?;
    if copied != size {
        return Err(KociError::Pull(format!(
            "demux truncated entry {name}: copied {copied} of {size} bytes"
        )));
    }

    Ok(())
}

fn drain(reader: &mut dyn Read) -> Result<()> {
    std::io::copy(reader, &mut std::io::sink())?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;

    use super::*;

    const FIRST: &[u8] = b"first";
    const SECOND: &[u8] = b"second";
    const GONE: &[u8] = b"gone";

    fn route(demux: &mut Demux<Vec<u8>>, path: &str, size: u64, bytes: &'static [u8]) {
        let mut cursor = Cursor::new(bytes);

        demux
            .route(FileEntry {
                path: path.to_owned(),
                size,
                mode: 0o644,
                reader: &mut cursor,
            })
            .expect("route entry");
    }

    fn writer_of<'a>(demux: &'a Demux<Vec<u8>>, name: &str) -> &'a Vec<u8> {
        &demux.targets.get(name).expect("target").writer
    }

    fn route_expect_err(demux: &mut Demux<Vec<u8>>, size: u64, needle: &str) {
        let mut cursor = Cursor::new(FIRST);
        let error = demux
            .route(FileEntry {
                path: "a".to_owned(),
                size,
                mode: 0o644,
                reader: &mut cursor,
            })
            .expect_err("guard");

        assert!(
            error.to_string().contains(needle),
            "the guard must report the failure: {error}"
        );
    }

    #[test]
    fn demux_routes_matching_entries_and_drains_others() {
        // ARRANGE
        let mut demux = Demux::by_name(HashMap::from([
            ("a".to_owned(), Vec::new()),
            ("b".to_owned(), Vec::new()),
        ]));

        // ACT
        route(&mut demux, "a", 5, FIRST);
        route(&mut demux, "x/unknown", 4, GONE);
        route(&mut demux, "b", 6, SECOND);

        // ASSERT
        assert_eq!(writer_of(&demux, "a"), b"first");
        assert_eq!(writer_of(&demux, "b"), b"second");
    }

    #[test]
    fn demux_fails_fast_when_declared_size_differs_from_annotated() {
        // ARRANGE
        let mut demux = Demux::by_name(HashMap::from([("a".to_owned(), Vec::new())]))
            .with_sizes([("a".to_owned(), 99_u64)]);

        // ACT / ASSERT
        route_expect_err(&mut demux, 5, "annotated 99");
        assert!(writer_of(&demux, "a").is_empty(), "nothing was copied");
    }

    #[test]
    fn demux_reports_entries_shorter_than_declared() {
        // ARRANGE
        let mut demux = Demux::by_name(HashMap::from([("a".to_owned(), Vec::new())]));

        // ACT / ASSERT
        route_expect_err(&mut demux, 99, "copied 5 of 99");
    }
}
