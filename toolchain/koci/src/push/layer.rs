//! Deterministic eStargz layer assembly for pushed images.

use std::fs::File;
use std::os::unix::fs::MetadataExt as _;

use super::Entry;
use crate::error::Result;
use crate::stargz::{self, LayerBlob, Source};

/// Build the deterministic eStargz layer blob holding all entries.
///
/// # Errors
///
/// Returns an error when a source file cannot be read or archiving fails.
pub(crate) fn build(entries: &[Entry]) -> Result<LayerBlob> {
    let packed: Vec<stargz::Entry> = entries
        .iter()
        .map(|entry| stargz::Entry {
            path: entry.path.clone(),
            mode: entry_mode(&entry.source),
            source: Source::File(entry.source.clone()),
        })
        .collect();

    stargz::layer(&packed)
}

fn entry_mode(path: &std::path::Path) -> u32 {
    File::open(path)
        .ok()
        .and_then(|file| file.metadata().ok())
        .map_or(0o644, |meta| meta.mode() & 0o777)
}

#[cfg(test)]
mod tests {
    use flate2::read::MultiGzDecoder;
    use tar::Archive;
    use tempfile::TempDir;

    use super::*;
    use crate::stargz::LayerBlob;

    fn fixture(names: &[(&str, &[u8])]) -> (TempDir, Vec<Entry>) {
        let dir = TempDir::new().expect("temp dir");
        let entries = names
            .iter()
            .map(|&(name, contents)| {
                let source = dir.path().join(format!("{name}.file"));
                std::fs::create_dir_all(source.parent().expect("parent exists"))
                    .expect("create parent");
                std::fs::write(&source, contents).expect("write entry");

                Entry {
                    path: name.to_owned(),
                    source,
                }
            })
            .collect();

        (dir, entries)
    }

    fn archive_paths(layer: &LayerBlob) -> Vec<String> {
        let stream = MultiGzDecoder::new(layer.bytes.as_slice());
        Archive::new(stream)
            .entries()
            .expect("tar entries")
            .filter_map(std::result::Result::ok)
            .map(|entry| String::from_utf8_lossy(entry.path_bytes().as_ref()).into_owned())
            .collect()
    }

    #[test]
    fn layer_digest_is_deterministic_across_input_order() {
        // ARRANGE
        let one = [
            ("board/cmdline", "console=ttyS0".as_bytes()),
            ("board/kernel.bin", b"kernel bytes".as_slice()),
            ("board/modules/m.ko", b"module".as_slice()),
        ];
        let (dir, first) = fixture(&one);
        let (rev_dir, second) = {
            let reversed: Vec<(&str, &[u8])> = one.iter().rev().copied().collect();
            let built = fixture(&reversed);
            (built.0, built.1)
        };

        // ACT
        let packed_first = build(&first).expect("build layer");
        let packed_second = build(&second).expect("build reversed layer");

        // ASSERT
        assert_eq!(packed_first.digest, packed_second.digest);
        assert_eq!(packed_first.toc_digest, packed_second.toc_digest);
        assert_eq!(packed_first.uncompressed, packed_second.uncompressed);
        drop(dir);
        drop(rev_dir);
    }

    #[test]
    fn layer_stores_entries_sorted_by_path() {
        // ARRANGE
        let names = [
            ("board/cmdline", b"console=ttyS0".as_slice()),
            ("board/in/nested.txt", b"nested".as_slice()),
            ("board/kernel.bin", b"kernel".as_slice()),
        ];
        let (dir, entries) = fixture(&names);

        // ACT
        let packed = build(&entries).expect("build layer");
        let paths = archive_paths(&packed);
        drop(dir);

        // ASSERT
        let mut sorted = paths.clone();
        sorted.sort_unstable();
        assert_eq!(paths, sorted);
        assert!(paths.contains(&".no.prefetch.landmark".to_owned()));
    }
}
