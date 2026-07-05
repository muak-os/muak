//! Rootfs staging and EROFS compression.

use std::os::unix::fs as unix_fs;
use std::path::Path;

use ::erofs::layout::ImagePlan;
use ::erofs::{FileContexts, MkfsConfig};

use crate::erofs;
use crate::error::{RamuneError, Result};

const REQUIRED_DIRS: &[&str] = &["dev", "proc", "sys", "run", "etc/services", "etc/selinux"];

/// Stages a rootfs directory and plans the EROFS image layout.
///
/// # Errors
///
/// Returns an error when copying the rootfs, creating required directories or planning the EROFS image fails.
pub fn prepare_and_plan<'a>(
    rootfs_dir: &Path,
    file_contexts: Option<&'a FileContexts>,
    rootfs_compression_level: i32,
) -> Result<(ImagePlan, MkfsConfig<'a>, u64)> {
    let parent = rootfs_dir.parent().unwrap_or(rootfs_dir);
    let staging = tempfile::Builder::new()
        .tempdir_in(parent)
        .map_err(|source| RamuneError::WriteError {
            file: parent.display().to_string(),
            source,
        })?;

    copy_dir_all(rootfs_dir, staging.path())?;
    inject_required_dirs(staging.path())?;
    ensure_default_resolv_conf(&staging.path().join("etc/resolv.conf"))?;

    erofs::plan_image(staging.path(), file_contexts, rootfs_compression_level)
}

fn copy_dir_all(src: &Path, dst: &Path) -> Result<()> {
    std::fs::create_dir_all(dst).map_err(|source| RamuneError::WriteError {
        file: dst.display().to_string(),
        source,
    })?;

    let entries = std::fs::read_dir(src).map_err(|source| RamuneError::ReadError {
        file: src.display().to_string(),
        source,
    })?;

    for entry in entries {
        let entry = entry.map_err(|source| RamuneError::ReadError {
            file: src.display().to_string(),
            source,
        })?;
        copy_path(&entry.path(), &dst.join(entry.file_name()))?;
    }

    Ok(())
}

fn copy_path(src: &Path, dst: &Path) -> Result<()> {
    let metadata = std::fs::symlink_metadata(src).map_err(|source| RamuneError::ReadError {
        file: src.display().to_string(),
        source,
    })?;

    if metadata.is_symlink() {
        copy_symlink(src, dst)
    } else if metadata.is_dir() {
        copy_dir_all(src, dst)
    } else {
        std::fs::copy(src, dst)
            .map(|_| ())
            .map_err(|source| RamuneError::WriteError {
                file: dst.display().to_string(),
                source,
            })
    }
}

fn copy_symlink(src: &Path, dst: &Path) -> Result<()> {
    let target = std::fs::read_link(src).map_err(|source| RamuneError::ReadError {
        file: src.display().to_string(),
        source,
    })?;

    unix_fs::symlink(&target, dst).map_err(|source| RamuneError::WriteError {
        file: dst.display().to_string(),
        source,
    })
}

fn inject_required_dirs(root: &Path) -> Result<()> {
    for dir in REQUIRED_DIRS {
        let path = root.join(dir);
        std::fs::create_dir_all(&path).map_err(|source| RamuneError::WriteError {
            file: path.display().to_string(),
            source,
        })?;
    }

    Ok(())
}

fn ensure_default_resolv_conf(path: &Path) -> Result<()> {
    if std::fs::symlink_metadata(path).is_err() {
        unix_fs::symlink(Path::new("/run/resolv.conf"), path).map_err(|source| {
            RamuneError::WriteError {
                file: path.display().to_string(),
                source,
            }
        })?;
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use ::erofs::writer;

    use super::*;

    fn prepare_image(
        rootfs_dir: &Path,
        fc: Option<&::erofs::FileContexts>,
        clevel: i32,
    ) -> Vec<u8> {
        let (plan, config, _size) =
            prepare_and_plan(rootfs_dir, fc, clevel).expect("prepare_and_plan");
        let mut buf = Vec::new();
        writer::image(&mut buf, &plan, &config).expect("image");

        buf
    }

    #[test]
    fn prepare_rootfs_copies_existing_files() {
        // ARRANGE
        let tmp = tempfile::tempdir().expect("tempdir");
        let rootfs = tmp.path().join("rootfs");
        std::fs::create_dir_all(rootfs.join("sbin")).expect("mkdir");
        std::fs::write(rootfs.join("sbin/init"), b"binary").expect("write");

        // ACT
        let erofs = prepare_image(&rootfs, None, 3);

        // ASSERT
        assert!(!erofs.is_empty());
    }

    #[test]
    fn copy_dir_all_preserves_symlinks() {
        // ARRANGE
        let tmp = tempfile::tempdir().expect("tempdir");
        let src = tmp.path().join("src");
        std::fs::create_dir_all(src.join("bin")).expect("mkdir src");
        std::fs::write(src.join("bin/tool"), b"binary").expect("write");
        unix_fs::symlink("bin/tool", src.join("init")).expect("symlink");
        let dst = tmp.path().join("dst");

        // ACT
        copy_dir_all(&src, &dst).expect("copy_dir_all");

        // ASSERT
        assert_eq!(
            std::fs::read_link(dst.join("init")).expect("read_link"),
            std::path::Path::new("bin/tool")
        );
        assert_eq!(
            std::fs::read(dst.join("bin/tool")).expect("read"),
            b"binary"
        );
    }

    #[test]
    fn copy_dir_all_missing_source_errors() {
        // ARRANGE
        let tmp = tempfile::tempdir().expect("tempdir");
        let src = tmp.path().join("missing");
        let dst = tmp.path().join("dst");

        // ACT
        let result = copy_dir_all(&src, &dst);

        // ASSERT
        assert!(
            result
                .as_ref()
                .is_err_and(|error| matches!(error, RamuneError::ReadError { .. }))
        );
    }

    #[test]
    fn prepare_rootfs_missing_rootfs_errors() {
        // ARRANGE
        let tmp = tempfile::tempdir().expect("tempdir");
        let rootfs = tmp.path().join("missing-rootfs");

        // ACT
        let result = prepare_and_plan(&rootfs, None, 3);

        // ASSERT
        assert!(
            result
                .as_ref()
                .is_err_and(|error| matches!(error, RamuneError::ReadError { .. }))
        );
    }

    #[test]
    fn copy_path_missing_source_errors() {
        // ARRANGE
        let tmp = tempfile::tempdir().expect("tempdir");
        let src = tmp.path().join("missing");
        let dst = tmp.path().join("dst");

        // ACT
        let result = copy_path(&src, &dst);

        // ASSERT
        assert!(
            result
                .as_ref()
                .is_err_and(|error| matches!(error, RamuneError::ReadError { .. }))
        );
    }

    #[test]
    fn copy_symlink_non_symlink_source_errors() {
        // ARRANGE
        let tmp = tempfile::tempdir().expect("tempdir");
        let src = tmp.path().join("file");
        std::fs::write(&src, b"data").expect("write");
        let dst = tmp.path().join("dst");

        // ACT
        let result = copy_symlink(&src, &dst);

        // ASSERT
        assert!(
            result
                .as_ref()
                .is_err_and(|error| matches!(error, RamuneError::ReadError { .. }))
        );
    }

    #[test]
    fn copy_symlink_missing_parent_errors() {
        // ARRANGE
        let tmp = tempfile::tempdir().expect("tempdir");
        let src = tmp.path().join("link");
        unix_fs::symlink("target", &src).expect("symlink");
        let dst = tmp.path().join("missing/dst");

        // ACT
        let result = copy_symlink(&src, &dst);

        // ASSERT
        assert!(
            result
                .as_ref()
                .is_err_and(|error| matches!(error, RamuneError::WriteError { .. }))
        );
    }

    #[test]
    fn copy_path_regular_file_missing_parent_errors() {
        // ARRANGE
        let tmp = tempfile::tempdir().expect("tempdir");
        let src = tmp.path().join("source");
        std::fs::write(&src, b"data").expect("write");
        let dst = tmp.path().join("missing/dst");

        // ACT
        let result = copy_path(&src, &dst);

        // ASSERT
        assert!(
            result
                .as_ref()
                .is_err_and(|error| matches!(error, RamuneError::WriteError { .. }))
        );
    }

    #[test]
    fn copy_dir_all_missing_destination_parent_errors() {
        // ARRANGE
        let tmp = tempfile::tempdir().expect("tempdir");
        let src = tmp.path().join("src");
        std::fs::create_dir_all(&src).expect("mkdir src");
        let blocked = tmp.path().join("blocked");
        std::fs::write(&blocked, b"not a directory").expect("write blocked");
        let dst = blocked.join("dst");

        // ACT
        let result = copy_dir_all(&src, &dst);

        // ASSERT
        assert!(
            result
                .as_ref()
                .is_err_and(|error| matches!(error, RamuneError::WriteError { .. }))
        );
    }

    #[test]
    fn prepare_rootfs_parent_not_directory_errors() {
        // ARRANGE
        let tmp = tempfile::tempdir().expect("tempdir");
        let parent = tmp.path().join("parent");
        std::fs::write(&parent, b"not a directory").expect("write");
        let rootfs = parent.join("rootfs");

        // ACT
        let result = prepare_and_plan(&rootfs, None, 3);

        // ASSERT
        assert!(
            result
                .as_ref()
                .is_err_and(|error| matches!(error, RamuneError::WriteError { .. }))
        );
    }

    #[test]
    fn prepare_rootfs_required_dir_conflict_errors() {
        // ARRANGE
        let tmp = tempfile::tempdir().expect("tempdir");
        let rootfs = tmp.path().join("rootfs");
        std::fs::create_dir_all(&rootfs).expect("mkdir");
        std::fs::write(rootfs.join("etc"), b"not a directory").expect("write");

        // ACT
        let result = prepare_and_plan(&rootfs, None, 3);

        // ASSERT
        assert!(
            result
                .as_ref()
                .is_err_and(|error| matches!(error, RamuneError::WriteError { .. }))
        );
    }

    #[test]
    fn ensure_default_resolv_conf_missing_parent_errors() {
        // ARRANGE
        let tmp = tempfile::tempdir().expect("tempdir");
        let resolv = tmp.path().join("missing/resolv.conf");

        // ACT
        let result = ensure_default_resolv_conf(&resolv);

        // ASSERT
        assert!(
            result
                .as_ref()
                .is_err_and(|error| matches!(error, RamuneError::WriteError { .. }))
        );
    }
}
