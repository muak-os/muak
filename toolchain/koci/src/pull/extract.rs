//! Materializing streamed image entries into a directory tree.

use std::fs;
use std::os::unix::fs as unix_fs;
use std::os::unix::fs::PermissionsExt as _;
use std::path::Path;

use rustix::fs::{AtFlags, CWD, Gid, Timespec, Timestamps, Uid};

use super::entries::{FileEntry, Kind, Meta};
use crate::error::{KociError, Result};

/// Write one streamed entry under `root`, replacing any previous path.
///
/// # Errors
///
/// Returns an error when a parent directory or link cannot be created, the
/// streamed bytes cannot be copied, or metadata cannot be applied.
pub fn write(mut entry: FileEntry<'_>, root: &Path) -> Result<()> {
    let path = root.join(&entry.path);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    remove_existing(&path)?;

    match entry.kind {
        Kind::File => write_file(&mut entry, &path),
        Kind::Symlink => write_symlink(&entry, &path),
        Kind::HardLink => write_hard_link(&entry, root, &path),
    }
}

fn write_file(entry: &mut FileEntry<'_>, path: &Path) -> Result<()> {
    let mut file = fs::File::create_new(path)?;
    std::io::copy(&mut entry.reader, &mut file)?;

    apply_meta(path, &entry.meta, AtFlags::empty())
}

fn write_symlink(entry: &FileEntry<'_>, path: &Path) -> Result<()> {
    let target = entry
        .link
        .as_deref()
        .ok_or_else(|| missing_link(&entry.path))?;
    unix_fs::symlink(target, path)?;

    apply_meta(path, &entry.meta, AtFlags::SYMLINK_NOFOLLOW)
}

fn write_hard_link(entry: &FileEntry<'_>, root: &Path, path: &Path) -> Result<()> {
    let target = entry
        .link
        .as_deref()
        .ok_or_else(|| missing_link(&entry.path))?;
    fs::hard_link(root.join(target), path)?;

    Ok(())
}

fn apply_meta(path: &Path, meta: &Meta, flags: AtFlags) -> Result<()> {
    set_owner(path, meta, flags)?;
    if flags.is_empty() {
        fs::set_permissions(path, fs::Permissions::from_mode(meta.mode & 0o7777))?;
    }

    set_times(path, meta.mtime, flags)
}

fn set_owner(path: &Path, meta: &Meta, flags: AtFlags) -> Result<()> {
    let owner = Some(Uid::from_raw(meta.uid));
    let group = Some(Gid::from_raw(meta.gid));
    match rustix::fs::chownat(CWD, path, owner, group, flags) {
        Ok(()) | Err(rustix::io::Errno::PERM) => Ok(()),
        Err(error) => Err(KociError::IoError(error.into())),
    }
}

fn set_times(path: &Path, mtime: u64, flags: AtFlags) -> Result<()> {
    let time = Timespec {
        tv_sec: i64::try_from(mtime).unwrap_or(i64::MAX),
        tv_nsec: 0,
    };
    let times = Timestamps {
        last_access: time,
        last_modification: time,
    };

    rustix::fs::utimensat(CWD, path, &times, flags)
        .map_err(|error| KociError::IoError(error.into()))
}

fn remove_existing(path: &Path) -> Result<()> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.is_dir() => fs::remove_dir_all(path).map_err(KociError::IoError),
        Ok(_) => fs::remove_file(path).map_err(KociError::IoError),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(KociError::IoError(error)),
    }
}

fn missing_link(path: &str) -> KociError {
    KociError::LayerExtractionError(format!("Link entry without a target: {path}"))
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;
    use std::os::unix::fs::MetadataExt as _;
    use std::time::{Duration, UNIX_EPOCH};

    use tempfile::TempDir;

    use super::*;

    fn file_meta() -> Meta {
        Meta {
            mode: 0o644,
            ..Meta::default()
        }
    }

    fn entry<'a>(
        path: &str,
        kind: Kind,
        link: Option<&str>,
        meta: Meta,
        reader: &'a mut dyn std::io::Read,
    ) -> FileEntry<'a> {
        FileEntry {
            path: path.to_owned(),
            kind,
            size: 0,
            link: link.map(str::to_owned),
            meta,
            reader,
        }
    }

    #[test]
    fn write_creates_nested_files_with_mode_and_mtime() {
        // ARRANGE
        let root = TempDir::new().expect("temp dir");
        let meta = Meta {
            mode: 0o640,
            uid: 0,
            gid: 0,
            mtime: 1_700_000_000,
        };
        let mut reader = Cursor::new(b"payload".to_vec());

        // ACT
        write(
            entry("etc/motd", Kind::File, None, meta, &mut reader),
            root.path(),
        )
        .expect("write file");

        // ASSERT
        let path = root.path().join("etc/motd");
        assert_eq!(fs::read(&path).expect("read file"), b"payload");
        let written = fs::metadata(&path).expect("stat file");
        assert_eq!(written.permissions().mode() & 0o7777, 0o640);
        let elapsed = written
            .modified()
            .expect("mtime")
            .duration_since(UNIX_EPOCH)
            .expect("mtime after epoch");
        assert_eq!(elapsed, Duration::from_secs(1_700_000_000));
    }

    #[test]
    fn write_replaces_a_previous_symlink_with_a_file() {
        // ARRANGE
        let root = TempDir::new().expect("temp dir");
        fs::write(root.path().join("real"), b"real").expect("seed target");
        unix_fs::symlink("real", root.path().join("link")).expect("seed symlink");
        let mut reader = Cursor::new(b"replaced".to_vec());

        // ACT
        write(
            entry("link", Kind::File, None, file_meta(), &mut reader),
            root.path(),
        )
        .expect("write file");

        // ASSERT
        assert_eq!(
            fs::read(root.path().join("link")).expect("read file"),
            b"replaced"
        );
        assert_eq!(
            fs::read(root.path().join("real")).expect("read target"),
            b"real"
        );
    }

    #[test]
    fn write_creates_symlinks_with_their_target() {
        // ARRANGE
        let root = TempDir::new().expect("temp dir");
        let mut reader = Cursor::new(Vec::new());

        // ACT
        write(
            entry(
                "etc/resolv.conf",
                Kind::Symlink,
                Some("/run/resolv.conf"),
                Meta::default(),
                &mut reader,
            ),
            root.path(),
        )
        .expect("write symlink");

        // ASSERT
        let target = fs::read_link(root.path().join("etc/resolv.conf")).expect("read link");
        assert_eq!(target, Path::new("/run/resolv.conf"));
    }

    #[test]
    fn write_creates_hard_links_to_extracted_targets() {
        // ARRANGE
        let root = TempDir::new().expect("temp dir");
        let mut reader = Cursor::new(b"shared".to_vec());
        write(
            entry("etc/motd", Kind::File, None, file_meta(), &mut reader),
            root.path(),
        )
        .expect("write target");
        let mut link_reader = Cursor::new(Vec::new());

        // ACT
        write(
            entry(
                "etc/motd-link",
                Kind::HardLink,
                Some("etc/motd"),
                Meta::default(),
                &mut link_reader,
            ),
            root.path(),
        )
        .expect("write hard link");

        // ASSERT
        let target = fs::metadata(root.path().join("etc/motd")).expect("stat target");
        let link = fs::metadata(root.path().join("etc/motd-link")).expect("stat link");
        assert_eq!(link.ino(), target.ino());
    }

    #[test]
    fn write_reports_links_without_targets() {
        // ARRANGE
        let root = TempDir::new().expect("temp dir");
        let mut reader = Cursor::new(Vec::new());

        // ACT
        let error = write(
            entry("link", Kind::Symlink, None, Meta::default(), &mut reader),
            root.path(),
        )
        .expect_err("missing target must fail");

        // ASSERT
        assert!(error.to_string().contains("without a target"));
    }
}
