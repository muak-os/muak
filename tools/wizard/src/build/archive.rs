//! Initramfs archive building.

use std::collections::HashMap;
use std::io::Write;

use ramune::Entry;
use ramune::archive;
use ramune::error::RamuneError;

use crate::error::{Result, WizardError};
use crate::source::extension::{BufferedReader, Metadata as ExtensionMetadata};

/// Builds an extension metadata and buffered file data.
///
/// # Errors
///
/// Returns an error when the extension image cannot be planned.
pub(crate) fn build_extension_image(
    name: &str,
    meta: &ExtensionMetadata,
    reader: &mut BufferedReader,
) -> Result<mumi::image::Image> {
    let entries: Vec<mumi::image::Entry> = meta
        .files
        .iter()
        .map(|file_entry| {
            let &(ref path, size, mode) = file_entry;
            mumi::image::Entry {
                path: format!("/{path}"),
                size,
                mode: 0o100_000 | mode,
                symlink_target: Vec::new(),
            }
        })
        .collect();
    let config = mumi::image::BuildConfig {
        compression_level: mumi::DEFAULT_ZSTD_COMPRESSION_LEVEL,
        file_contexts: None,
    };

    mumi::image::build(name, &entries, reader, &config)
        .map_err(|e| WizardError::BuildError(format!("build extension {} image: {e}", meta.name)))
}

/// Writes a tail archive of extension images plus the profile into `writer`.
///
/// # Errors
///
/// Returns an error when CPIO validation fails or a data callback fails.
pub(crate) fn build_tail(
    images_by_path: &HashMap<String, mumi::image::Image>,
    entries: &mut [Entry],
    profile_bytes: &[u8],
    writer: &mut impl Write,
) -> Result<()> {
    archive::cpio(entries, writer, |entry, out| {
        if entry.path == "profile.toml" {
            return out
                .write_all(profile_bytes)
                .map_err(|e| RamuneError::WriteError {
                    file: entry.path.clone(),
                    source: e,
                });
        }
        let image = images_by_path
            .get(&entry.path)
            .ok_or_else(|| RamuneError::CpioError(format!("unknown tail entry: {}", entry.path)))?;
        image
            .write(out)
            .map_err(|e| RamuneError::CpioError(e.to_string()))
    })
    .map(|_| ())
    .map_err(|e| WizardError::BuildError(format!("build initramfs tail: {e}")))
}

/// Computes the exact size of the tail archive from its entries.
#[must_use]
pub(crate) fn tail_exact_size(entries: &[Entry]) -> u64 {
    archive::size(entries)
}

/// Sanitizes a logical extension name into an archive-safe path segment.
pub(crate) fn extension_archive_name(name: &str) -> String {
    name.replace('/', "-")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn image_and_path(name: &str) -> (mumi::image::Image, String) {
        let meta = ExtensionMetadata {
            name: name.to_owned(),
            files: vec![("usr/bin/tool".to_owned(), 5, 0o755)],
        };
        let mut reader = BufferedReader::new(vec![b"hello".to_vec()]);
        let image = build_extension_image(name, &meta, &mut reader).expect("build image");
        let path = format!("extensions/{}.erofs", extension_archive_name(name));
        (image, path)
    }

    fn tail_parts() -> (
        HashMap<String, mumi::image::Image>,
        Vec<Entry>,
        &'static [u8],
    ) {
        let (image, path) = image_and_path("muak-os/test");
        let mut images_by_path = HashMap::new();
        images_by_path.insert(path.clone(), image);
        let mut entries = vec![Entry {
            path: path.clone(),
            mode: 0o100_644,
            len: images_by_path.get(&path).unwrap().len(),
        }];
        entries.push(Entry {
            path: "profile.toml".to_owned(),
            mode: 0o100_644,
            len: 15,
        });
        (images_by_path, entries, b"profile = true\n")
    }

    #[test]
    fn derives_stable_archive_names() {
        // ARRANGE / ACT / ASSERT
        assert_eq!(extension_archive_name("muak-os/qemu"), "muak-os-qemu");
        assert_eq!(
            extension_archive_name("muak-os/iscsi-tools"),
            "muak-os-iscsi-tools"
        );
    }

    #[test]
    fn build_tail_writes_archive() {
        // ARRANGE
        let (images_by_path, mut entries, profile) = tail_parts();

        // ACT
        let mut buf = Vec::new();
        build_tail(&images_by_path, &mut entries, profile, &mut buf).expect("build tail");

        // ASSERT
        assert!(!buf.is_empty());
    }

    #[test]
    fn tail_exact_size_matches_built_size() {
        // ARRANGE
        let (images_by_path, mut entries, profile) = tail_parts();
        let expected = tail_exact_size(&entries);

        // ACT
        let mut buf = Vec::new();
        build_tail(&images_by_path, &mut entries, profile, &mut buf).expect("build tail");

        // ASSERT
        assert_eq!(expected, u64::try_from(buf.len()).unwrap_or(0));
    }

    #[test]
    fn build_tail_empty_returns_ok() {
        // ARRANGE
        let images_by_path = HashMap::new();
        let mut entries: Vec<Entry> = vec![];
        let mut buf = Vec::new();

        // ACT
        build_tail(&images_by_path, &mut entries, b"", &mut buf).expect("build empty tail");

        // ASSERT
        assert!(buf.is_empty());
    }
}
