//! Scanning and processing OCI layer tar archives.

use alloc::borrow::Cow;
use std::io::Read;
use std::path::{Path, PathBuf};

use super::entries::{FileEntry, Kind, Meta};
use super::paths::{normalize_entry_path_into, whiteout_target_into};
use crate::error::{KociError, Result};
use crate::stargz;

/// Reused buffers for classifying tar entries without per-entry allocation.
pub(crate) struct Scratch {
    path: PathBuf,
    link: PathBuf,
}

impl Scratch {
    /// Create empty reusable buffers.
    pub(crate) fn new() -> Self {
        Self {
            path: PathBuf::new(),
            link: PathBuf::new(),
        }
    }
}

/// Outcome of classifying a tar entry, borrowing the caller's reused buffers.
#[derive(Clone, Copy, Debug)]
pub(crate) enum EntryInfo<'a> {
    Skip,
    Whiteout(&'a Path),
    File(&'a Path, u64, Meta),
    Symlink(&'a Path, &'a Path, Meta),
    HardLink(&'a Path, &'a Path, Meta),
}

impl EntryInfo<'_> {
    /// The entry path, or `None` for entries that carry no path of their own.
    pub(crate) fn path(&self) -> Option<&Path> {
        match *self {
            Self::Skip | Self::Whiteout(_) => None,
            Self::File(path, ..) | Self::Symlink(path, ..) | Self::HardLink(path, ..) => Some(path),
        }
    }
}

/// Classify a tar entry into the reused `scratch` buffers.
///
/// # Errors
///
/// Returns an error when the entry path or hard link target escapes the
/// extraction root, or the entry type is unsupported.
pub(crate) fn classify_tar_entry<'a>(
    entry: &tar::Entry<impl Read>,
    scratch: &'a mut Scratch,
) -> Result<EntryInfo<'a>> {
    let header = entry.header();
    let entry_type = header.entry_type();

    if !normalize_entry_path_into(entry.path()?.as_ref(), &mut scratch.path)? {
        return Ok(EntryInfo::Skip);
    }

    if whiteout_target_into(&mut scratch.path) {
        return Ok(EntryInfo::Whiteout(&scratch.path));
    }

    if entry_type.is_dir() {
        return Ok(EntryInfo::Skip);
    }

    let meta = meta_of(header);

    if entry_type.is_symlink() {
        let target = link_name(entry, &scratch.path)?;
        let target = link_target_into(target.as_ref(), &mut scratch.link);
        return Ok(EntryInfo::Symlink(&scratch.path, target, meta));
    }

    if entry_type.is_hard_link() {
        let target = link_name(entry, &scratch.path)?;
        if !normalize_entry_path_into(target.as_ref(), &mut scratch.link)? {
            return Err(unsupported_entry(
                "hard link without a target",
                &scratch.path,
            ));
        }
        return Ok(EntryInfo::HardLink(&scratch.path, &scratch.link, meta));
    }

    if !entry_type.is_file() {
        return Err(KociError::LayerExtractionError(format!(
            "Unsupported OCI layer entry type for {}",
            scratch.path.display()
        )));
    }

    if stargz::member::is_bookkeeping(&path_string(&scratch.path)) {
        return Ok(EntryInfo::Skip);
    }

    Ok(EntryInfo::File(&scratch.path, entry.size(), meta))
}

/// Scan a single layer and collect its whiteout targets.
///
/// # Errors
///
/// Returns an error when an entry cannot be classified.
pub(crate) fn scan_whiteouts<R: Read>(data: R) -> Result<Vec<PathBuf>> {
    let mut whiteouts: Vec<PathBuf> = Vec::new();
    let mut scratch = Scratch::new();

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
    let (path, kind, size, link, meta) = match info {
        EntryInfo::Skip | EntryInfo::Whiteout(_) => return Ok(()),
        EntryInfo::File(path, size, meta) => (path, Kind::File, size, None, meta),
        EntryInfo::Symlink(path, target, meta) => (path, Kind::Symlink, 0, Some(target), meta),
        EntryInfo::HardLink(path, target, meta) => (path, Kind::HardLink, 0, Some(target), meta),
    };
    handler(FileEntry {
        path: path_string(path),
        kind,
        size,
        link: link.map(path_string),
        meta,
        reader: &mut entry,
    })
}

/// The entry path as a `String`, keeping valid UTF-8 exact and falling back to lossy conversion.
pub(crate) fn path_string(path: &Path) -> String {
    match std::str::from_utf8(path.as_os_str().as_encoded_bytes()) {
        Ok(valid) => valid.to_owned(),
        Err(_) => path.to_string_lossy().into_owned(),
    }
}

fn meta_of(header: &tar::Header) -> Meta {
    Meta {
        mode: header.mode().unwrap_or(0o644),
        uid: u32::try_from(header.uid().unwrap_or(0)).unwrap_or(0),
        gid: u32::try_from(header.gid().unwrap_or(0)).unwrap_or(0),
        mtime: header.mtime().unwrap_or(0),
    }
}

fn link_name<'a>(entry: &'a tar::Entry<impl Read>, path: &Path) -> Result<Cow<'a, Path>> {
    match entry.link_name()? {
        Some(target) => Ok(target),
        None => Err(unsupported_entry("link without a target", path)),
    }
}

fn link_target_into<'a>(target: &Path, buf: &'a mut PathBuf) -> &'a Path {
    buf.clear();
    buf.push(target);
    buf.as_path()
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

    use tar::{Archive, Builder, EntryType, Header};

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

    fn link_layer(entry_type: EntryType, path: &str, target: &str) -> Vec<u8> {
        let mut archive = Builder::new(Vec::new());
        let mut header = Header::new_gnu();
        header.set_entry_type(entry_type);
        header.set_size(0);
        header.set_mode(0o755);
        header.set_uid(1000);
        header.set_gid(1000);
        header.set_mtime(1234);
        header.set_link_name(target).expect("link name");
        header.set_cksum();
        archive
            .append_data(&mut header, path, std::io::empty())
            .expect("append link");

        archive.into_inner().expect("finish archive")
    }

    fn classify_one<R: Read>(entry: &tar::Entry<R>, scratch: &mut Scratch) -> String {
        match classify_tar_entry(entry, scratch).expect("classify entry") {
            EntryInfo::Skip => "skip".to_owned(),
            EntryInfo::Whiteout(path) => format!("whiteout:{}", path.display()),
            EntryInfo::File(path, size, meta) => format!(
                "file:{}:{size}:{:o}:{}:{}:{}",
                path.display(),
                meta.mode,
                meta.uid,
                meta.gid,
                meta.mtime
            ),
            EntryInfo::Symlink(path, target, _) => {
                format!("symlink:{}:{}", path.display(), target.display())
            }
            EntryInfo::HardLink(path, target, _) => {
                format!("hardlink:{}:{}", path.display(), target.display())
            }
        }
    }

    fn classify_all(layer: &[u8]) -> Vec<String> {
        let mut results = Vec::new();
        let mut scratch = Scratch::new();
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
        assert_eq!(
            classified.first().expect("first entry"),
            "file:etc/motd:5:644:0:0:0"
        );
        assert_eq!(
            classified.get(1).expect("second entry"),
            "whiteout:etc/gone"
        );
    }

    #[test]
    fn classify_tar_entry_accepts_symlinks_verbatim() {
        // ARRANGE
        let layer = link_layer(EntryType::Symlink, "etc/resolv.conf", "/run/resolv.conf");

        // ACT
        let classified = classify_all(&layer);

        // ASSERT
        assert_eq!(classified, vec!["symlink:etc/resolv.conf:/run/resolv.conf"]);
    }

    #[test]
    fn classify_tar_entry_normalizes_hard_link_targets() {
        // ARRANGE
        let layer = link_layer(EntryType::Link, "etc/motd-link", "./etc/motd");

        // ACT
        let classified = classify_all(&layer);

        // ASSERT
        assert_eq!(classified, vec!["hardlink:etc/motd-link:etc/motd"]);
    }

    #[test]
    fn classify_tar_entry_carries_full_metadata() {
        // ARRANGE
        let layer = link_layer(EntryType::Symlink, "link", "target");
        let mut scratch = Scratch::new();
        let mut archive = Archive::new(Cursor::new(layer));
        let entry = archive
            .entries()
            .expect("iterate entries")
            .next()
            .expect("one entry")
            .expect("read entry");

        // ACT
        let EntryInfo::Symlink(path, target, meta) =
            classify_tar_entry(&entry, &mut scratch).expect("classify entry")
        else {
            panic!("expected a symlink entry");
        };

        // ASSERT
        assert_eq!(path, Path::new("link"));
        assert_eq!(target, Path::new("target"));
        assert_eq!(
            meta,
            Meta {
                mode: 0o755,
                uid: 1000,
                gid: 1000,
                mtime: 1234,
            }
        );
    }

    #[test]
    fn classify_tar_entry_rejects_escaping_hard_link_targets() {
        // ARRANGE
        let layer = link_layer(EntryType::Link, "etc/passwd", "../../escape");
        let mut scratch = Scratch::new();
        let mut archive = Archive::new(Cursor::new(layer));
        let entry = archive
            .entries()
            .expect("iterate entries")
            .next()
            .expect("one entry")
            .expect("read entry");

        // ACT
        let error =
            classify_tar_entry(&entry, &mut scratch).expect_err("escaping target must fail");

        // ASSERT
        assert!(error.to_string().contains("escapes extraction root"));
    }

    #[test]
    fn handle_file_entry_carries_links_into_streamed_entries() {
        // ARRANGE
        let layer = link_layer(EntryType::Symlink, "etc/resolv.conf", "/run/resolv.conf");
        let mut archive = Archive::new(Cursor::new(layer));
        let entry = archive
            .entries()
            .expect("iterate entries")
            .next()
            .expect("one entry")
            .expect("read entry");
        let mut scratch = Scratch::new();
        let info = classify_tar_entry(&entry, &mut scratch).expect("classify entry");
        let mut seen: Option<(Kind, Option<String>)> = None;

        // ACT
        handle_file_entry(entry, info, &mut |file| {
            seen = Some((file.kind, file.link.clone()));

            Ok(())
        })
        .expect("handle entry");

        // ASSERT
        assert_eq!(
            seen,
            Some((Kind::Symlink, Some("/run/resolv.conf".to_owned())))
        );
    }
}
