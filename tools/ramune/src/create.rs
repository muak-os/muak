//! Base initramfs creation from an init binary and rootfs directory.

use std::os::unix::fs as unix_fs;
use std::path::Path;

use crate::cpio::{self, CpioEntry};
use crate::erofs;
use crate::error::{RamuneError, Result};

/// CPIO mode for regular executable files.
const MODE_EXEC: u32 = 0o100755;

/// CPIO mode for regular files.
const MODE_FILE: u32 = 0o100644;

/// Directories that must always exist in the rootfs.
const REQUIRED_DIRS: &[&str] = &["dev", "proc", "sys", "run", "etc/services", "etc/selinux"];

/// Configuration for base initramfs creation.
pub struct CreateConfig<'a> {
    pub init: &'a Path,
    pub rootfs_dir: &'a Path,
    pub file_contexts: Option<&'a ::erofs::FileContexts>,
    pub compression_level: i32,
}

/// Copies `src` into `dst` recursively, preserving symlinks as symlinks.
fn copy_dir_all(src: &Path, dst: &Path) -> Result<()> {
    std::fs::create_dir_all(dst).map_err(|e| RamuneError::WriteError {
        file: dst.display().to_string(),
        source: e,
    })?;
    for entry in std::fs::read_dir(src).map_err(|e| RamuneError::ReadError {
        file: src.display().to_string(),
        source: e,
    })? {
        let entry = entry.map_err(|e| RamuneError::ReadError {
            file: src.display().to_string(),
            source: e,
        })?;
        let src_path = entry.path();
        let dst_path = dst.join(entry.file_name());
        let meta = std::fs::symlink_metadata(&src_path).map_err(|e| RamuneError::ReadError {
            file: src_path.display().to_string(),
            source: e,
        })?;
        if meta.is_symlink() {
            let target = std::fs::read_link(&src_path).map_err(|e| RamuneError::ReadError {
                file: src_path.display().to_string(),
                source: e,
            })?;
            unix_fs::symlink(&target, &dst_path).map_err(|e| RamuneError::WriteError {
                file: dst_path.display().to_string(),
                source: e,
            })?;
        } else if meta.is_dir() {
            copy_dir_all(&src_path, &dst_path)?;
        } else {
            std::fs::copy(&src_path, &dst_path).map_err(|e| RamuneError::WriteError {
                file: dst_path.display().to_string(),
                source: e,
            })?;
        }
    }
    Ok(())
}

/// Stages rootfs into a temp dir, ensuring required dirs exists.
fn prepare_rootfs(rootfs_dir: &Path) -> Result<tempfile::TempDir> {
    let parent = rootfs_dir.parent().unwrap_or(rootfs_dir);
    let staging =
        tempfile::Builder::new()
            .tempdir_in(parent)
            .map_err(|e| RamuneError::WriteError {
                file: parent.display().to_string(),
                source: e,
            })?;
    copy_dir_all(rootfs_dir, staging.path())?;
    for dir in REQUIRED_DIRS {
        std::fs::create_dir_all(staging.path().join(dir)).map_err(|e| RamuneError::WriteError {
            file: dir.to_string(),
            source: e,
        })?;
    }
    let resolv = staging.path().join("etc/resolv.conf");
    if std::fs::symlink_metadata(&resolv).is_err() {
        unix_fs::symlink("/run/resolv.conf", &resolv).map_err(|e| RamuneError::WriteError {
            file: resolv.display().to_string(),
            source: e,
        })?;
    }
    Ok(staging)
}

/// Creates a base initramfs image from an init binary and rootfs directory.
pub(crate) fn create_initramfs(config: &CreateConfig<'_>) -> Result<Vec<u8>> {
    let init_data = std::fs::read(config.init).map_err(|e| RamuneError::ReadError {
        file: config.init.display().to_string(),
        source: e,
    })?;

    let staging = prepare_rootfs(config.rootfs_dir)?;
    let rootfs_erofs = erofs::create(staging.path(), config.file_contexts)?;

    let entries = vec![
        CpioEntry {
            path: "init".to_string(),
            mode: MODE_EXEC,
            data: init_data,
        },
        CpioEntry {
            path: "rootfs.erofs".to_string(),
            mode: MODE_FILE,
            data: rootfs_erofs,
        },
    ];

    let cpio_data = cpio::create_from_entries(&entries)?;
    zstd::encode_all(&cpio_data[..], config.compression_level)
        .map_err(|e| RamuneError::CpioError(format!("Compression failed: {e}")))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn setup_rootfs_dir(dir: &Path) {
        std::fs::create_dir_all(dir.join("sbin")).expect("mkdir");
        std::fs::write(dir.join("sbin/init"), b"init-binary").expect("write");
    }

    fn make_config<'a>(init: &'a Path, rootfs: &'a Path) -> CreateConfig<'a> {
        CreateConfig {
            init,
            rootfs_dir: rootfs,
            file_contexts: None,
            compression_level: 19,
        }
    }

    #[test]
    fn create_initramfs_produces_output() {
        // ARRANGE
        let tmp = tempfile::tempdir().expect("tempdir");
        let init_file = tmp.path().join("init");
        std::fs::write(&init_file, b"#!/bin/sh\nexec /sbin/init").expect("write init");
        let rootfs = tmp.path().join("rootfs");
        std::fs::create_dir(&rootfs).expect("mkdir rootfs");
        setup_rootfs_dir(&rootfs);

        // ACT
        let result = create_initramfs(&make_config(&init_file, &rootfs)).expect("create_initramfs");

        // ASSERT
        assert!(!result.is_empty());
    }

    #[test]
    fn create_initramfs_reproducible() {
        // ARRANGE
        let tmp = tempfile::tempdir().expect("tempdir");
        let init_file = tmp.path().join("init");
        std::fs::write(&init_file, b"init").expect("write init");
        let rootfs = tmp.path().join("rootfs");
        std::fs::create_dir(&rootfs).expect("mkdir rootfs");
        std::fs::write(rootfs.join("file"), b"data").expect("write");
        let config = make_config(&init_file, &rootfs);

        // ACT
        let img1 = create_initramfs(&config).expect("create 1");
        let img2 = create_initramfs(&config).expect("create 2");

        // ASSERT
        assert_eq!(img1, img2);
    }

    #[test]
    fn create_initramfs_missing_init_errors() {
        // ARRANGE
        let tmp = tempfile::tempdir().expect("tempdir");
        let rootfs = tmp.path().join("rootfs");
        std::fs::create_dir(&rootfs).expect("mkdir");

        // ACT
        let result = create_initramfs(&CreateConfig {
            init: Path::new("/nonexistent/init"),
            rootfs_dir: &rootfs,
            file_contexts: None,
            compression_level: 19,
        });

        // ASSERT
        assert!(matches!(result, Err(RamuneError::ReadError { .. })));
    }

    #[test]
    fn create_initramfs_with_file_contexts() {
        // ARRANGE
        let tmp = tempfile::tempdir().expect("tempdir");
        let init_file = tmp.path().join("init");
        std::fs::write(&init_file, b"init").expect("write init");
        let rootfs = tmp.path().join("rootfs");
        std::fs::create_dir(&rootfs).expect("mkdir rootfs");
        std::fs::write(rootfs.join("file"), b"data").expect("write");
        let fc =
            ::erofs::FileContexts::from_reader("/.*    system_u:object_r:file_t:s0\n".as_bytes())
                .expect("fc");

        // ACT
        let result = create_initramfs(&CreateConfig {
            init: &init_file,
            rootfs_dir: &rootfs,
            file_contexts: Some(&fc),
            compression_level: 19,
        })
        .expect("create_initramfs");

        // ASSERT
        assert!(!result.is_empty());
    }

    #[test]
    fn prepare_rootfs_injects_required_dirs() {
        // ARRANGE
        let tmp = tempfile::tempdir().expect("tempdir");
        let rootfs = tmp.path().join("rootfs");
        std::fs::create_dir(&rootfs).expect("mkdir");

        // ACT
        let staging = prepare_rootfs(&rootfs).expect("prepare_rootfs");

        // ASSERT
        for dir in REQUIRED_DIRS {
            assert!(staging.path().join(dir).is_dir(), "missing dir: {dir}");
        }
    }

    #[test]
    fn prepare_rootfs_creates_resolv_conf_symlink() {
        // ARRANGE
        let tmp = tempfile::tempdir().expect("tempdir");
        let rootfs = tmp.path().join("rootfs");
        std::fs::create_dir(&rootfs).expect("mkdir");

        // ACT
        let staging = prepare_rootfs(&rootfs).expect("prepare_rootfs");

        // ASSERT
        let resolv = staging.path().join("etc/resolv.conf");
        assert!(
            std::fs::symlink_metadata(&resolv).is_ok(),
            "resolv.conf missing"
        );
        assert_eq!(
            std::fs::read_link(&resolv).expect("read_link"),
            std::path::Path::new("/run/resolv.conf")
        );
    }

    #[test]
    fn prepare_rootfs_preserves_existing_resolv_conf() {
        // ARRANGE
        let tmp = tempfile::tempdir().expect("tempdir");
        let rootfs = tmp.path().join("rootfs");
        std::fs::create_dir_all(rootfs.join("etc")).expect("mkdir");
        unix_fs::symlink("/custom/resolv.conf", rootfs.join("etc/resolv.conf")).expect("symlink");

        // ACT
        let staging = prepare_rootfs(&rootfs).expect("prepare_rootfs");

        // ASSERT
        assert_eq!(
            std::fs::read_link(staging.path().join("etc/resolv.conf")).expect("read_link"),
            std::path::Path::new("/custom/resolv.conf")
        );
    }

    #[test]
    fn prepare_rootfs_copies_existing_files() {
        // ARRANGE
        let tmp = tempfile::tempdir().expect("tempdir");
        let rootfs = tmp.path().join("rootfs");
        std::fs::create_dir_all(rootfs.join("sbin")).expect("mkdir");
        std::fs::write(rootfs.join("sbin/init"), b"binary").expect("write");

        // ACT
        let staging = prepare_rootfs(&rootfs).expect("prepare_rootfs");

        // ASSERT
        assert_eq!(
            std::fs::read(staging.path().join("sbin/init")).expect("read"),
            b"binary"
        );
    }
}
