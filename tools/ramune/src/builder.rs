//! Base initramfs creation from an init binary and rootfs directory.

use std::os::unix::fs as unix_fs;
use std::path::Path;

use crate::compress;
use crate::cpio::{self, CpioEntry};
use crate::erofs;
use crate::error::{RamuneError, Result};

/// CPIO mode for regular executable files.
const MODE_EXEC: u32 = 0o100_755;

/// CPIO mode for regular files.
const MODE_FILE: u32 = 0o100_644;

/// Directories that must always exist in the rootfs.
const REQUIRED_DIRS: &[&str] = &["dev", "proc", "sys", "run", "etc/services", "etc/selinux"];

/// Configuration for base initramfs creation.
pub struct CreateConfig<'a> {
    /// Path to the init binary.
    pub init: &'a Path,
    /// Path to the rootfs directory to embed.
    pub rootfs_dir: &'a Path,
    /// Optional `SELinux` file contexts.
    pub file_contexts: Option<&'a ::erofs::FileContexts>,
    /// Zstd compression level for the output initramfs.
    pub compression_level: i32,
    /// Zstd compression level for the embedded rootfs.
    pub rootfs_compression_level: i32,
}

/// Creates a base initramfs image from an init binary and rootfs directory.
///
/// # Errors
///
/// Returns an error when reading inputs, building the staged rootfs, compressing the archive,
/// or writing the output image fails.
pub fn create(config: &CreateConfig<'_>, output: &Path) -> Result<()> {
    let init_data = std::fs::read(config.init).map_err(|e| RamuneError::ReadError {
        file: config.init.display().to_string(),
        source: e,
    })?;

    let rootfs_erofs = prepare_rootfs(config.rootfs_dir).and_then(|staging| {
        erofs::create(
            staging.path(),
            config.file_contexts,
            config.rootfs_compression_level,
        )
    })?;

    let entries = vec![
        CpioEntry {
            path: "init",
            mode: MODE_EXEC,
            data: init_data.as_slice(),
        },
        CpioEntry {
            path: "rootfs.erofs",
            mode: MODE_FILE,
            data: rootfs_erofs.as_slice(),
        },
    ];

    let mut encoder = compress::encoder(Vec::new(), config.compression_level)?;
    cpio::write_archive(&mut encoder, &entries)?;
    let data = encoder.finish().map_err(RamuneError::CompressionError)?;

    std::fs::write(output, &data).map_err(|source| RamuneError::WriteError {
        file: output.display().to_string(),
        source,
    })
}

/// Copies `src` into `dst` recursively, preserving symlinks as symlinks.
fn copy_dir_all(src: &Path, dst: &Path) -> Result<()> {
    std::fs::create_dir_all(dst).map_err(|source| RamuneError::WriteError {
        file: dst.display().to_string(),
        source,
    })?;

    let entries = std::fs::read_dir(src).map_err(|source| RamuneError::ReadError {
        file: src.display().to_string(),
        source,
    })?;

    copy_dir_entries(src, dst, entries)
}

fn copy_dir_entries<I>(src: &Path, dst: &Path, entries: I) -> Result<()>
where
    I: IntoIterator<Item = std::io::Result<std::fs::DirEntry>>,
{
    entries.into_iter().try_for_each(|entry| {
        let entry = entry.map_err(|source| RamuneError::ReadError {
            file: src.display().to_string(),
            source,
        })?;
        let src_path = entry.path();
        let dst_path = dst.join(entry.file_name());
        copy_path(&src_path, &dst_path)
    })
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

/// Stages rootfs into a temp dir, ensuring required dirs exists.
fn prepare_rootfs(rootfs_dir: &Path) -> Result<tempfile::TempDir> {
    let parent = rootfs_dir.parent().unwrap_or(rootfs_dir);
    let staging = tempfile::Builder::new()
        .tempdir_in(parent)
        .map_err(|source| RamuneError::WriteError {
            file: parent.display().to_string(),
            source,
        })?;

    copy_dir_all(rootfs_dir, staging.path())?;

    for dir in REQUIRED_DIRS {
        let path = staging.path().join(dir);
        std::fs::create_dir_all(&path).map_err(|source| RamuneError::WriteError {
            file: path.display().to_string(),
            source,
        })?;
    }

    let resolv = staging.path().join("etc/resolv.conf");
    ensure_default_resolv_conf(&resolv).map(|()| staging)
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
            rootfs_compression_level: 3,
        }
    }

    #[test]
    fn create_produces_output() {
        // ARRANGE
        let tmp = tempfile::tempdir().expect("tempdir");
        let init_file = tmp.path().join("init");
        std::fs::write(&init_file, b"#!/bin/sh\nexec /sbin/init").expect("write init");
        let rootfs = tmp.path().join("rootfs");
        std::fs::create_dir_all(&rootfs).expect("mkdir rootfs");
        setup_rootfs_dir(&rootfs);

        // ACT
        let output = tmp.path().join("initramfs.img");
        create(&make_config(&init_file, &rootfs), &output).expect("create");

        // ASSERT
        let result = std::fs::read(output).expect("read output");
        assert!(!result.is_empty());
    }

    #[test]
    fn create_reproducible() {
        // ARRANGE
        let tmp = tempfile::tempdir().expect("tempdir");
        let init_file = tmp.path().join("init");
        std::fs::write(&init_file, b"init").expect("write init");
        let rootfs = tmp.path().join("rootfs");
        std::fs::create_dir_all(&rootfs).expect("mkdir rootfs");
        std::fs::write(rootfs.join("file"), b"data").expect("write");
        let config = make_config(&init_file, &rootfs);

        let output1 = tmp.path().join("initramfs-1.img");
        let output2 = tmp.path().join("initramfs-2.img");
        create(&config, &output1).expect("create 1");
        create(&config, &output2).expect("create 2");

        // ASSERT
        let img1 = std::fs::read(output1).expect("read output 1");
        let img2 = std::fs::read(output2).expect("read output 2");
        assert_eq!(img1, img2);
    }

    #[test]
    fn create_missing_init_errors() {
        // ARRANGE
        let tmp = tempfile::tempdir().expect("tempdir");
        let rootfs = tmp.path().join("rootfs");
        std::fs::create_dir_all(&rootfs).expect("mkdir");

        // ACT
        let output = tmp.path().join("initramfs.img");
        let result = create(
            &CreateConfig {
                init: Path::new("/nonexistent/init"),
                rootfs_dir: &rootfs,
                file_contexts: None,
                compression_level: 19,
                rootfs_compression_level: 3,
            },
            &output,
        );

        // ASSERT
        assert!(
            result
                .as_ref()
                .is_err_and(|error| matches!(error, RamuneError::ReadError { .. }))
        );
    }

    #[test]
    fn create_missing_rootfs_errors() {
        // ARRANGE
        let tmp = tempfile::tempdir().expect("tempdir");
        let init_file = tmp.path().join("init");
        std::fs::write(&init_file, b"init").expect("write init");
        let rootfs = tmp.path().join("missing-rootfs");

        // ACT
        let output = tmp.path().join("initramfs.img");
        let result = create(
            &CreateConfig {
                init: &init_file,
                rootfs_dir: &rootfs,
                file_contexts: None,
                compression_level: 19,
                rootfs_compression_level: 3,
            },
            &output,
        );

        // ASSERT
        assert!(
            result
                .as_ref()
                .is_err_and(|error| matches!(error, RamuneError::ReadError { .. }))
        );
    }

    #[test]
    fn create_with_file_contexts() {
        // ARRANGE
        let tmp = tempfile::tempdir().expect("tempdir");
        let init_file = tmp.path().join("init");
        std::fs::write(&init_file, b"init").expect("write init");
        let rootfs = tmp.path().join("rootfs");
        std::fs::create_dir_all(&rootfs).expect("mkdir rootfs");
        std::fs::write(rootfs.join("file"), b"data").expect("write");
        let fc =
            ::erofs::FileContexts::from_reader("/.*    system_u:object_r:file_t:s0\n".as_bytes())
                .expect("fc");

        // ACT
        let output = tmp.path().join("initramfs.img");
        create(
            &CreateConfig {
                init: &init_file,
                rootfs_dir: &rootfs,
                file_contexts: Some(&fc),
                compression_level: 19,
                rootfs_compression_level: 3,
            },
            &output,
        )
        .expect("create");

        // ASSERT
        let result = std::fs::read(output).expect("read output");
        assert!(!result.is_empty());
    }

    #[test]
    fn create_invalid_compression_level_errors() {
        // ARRANGE
        let tmp = tempfile::tempdir().expect("tempdir");
        let init_file = tmp.path().join("init");
        std::fs::write(&init_file, b"init").expect("write init");
        let rootfs = tmp.path().join("rootfs");
        std::fs::create_dir_all(&rootfs).expect("mkdir rootfs");
        std::fs::write(rootfs.join("file"), b"data").expect("write");

        // ACT
        let output = tmp.path().join("initramfs.img");
        let result = create(
            &CreateConfig {
                init: &init_file,
                rootfs_dir: &rootfs,
                file_contexts: None,
                compression_level: i32::MAX,
                rootfs_compression_level: 3,
            },
            &output,
        );

        // ASSERT
        assert!(
            result
                .as_ref()
                .is_err_and(|error| matches!(error, RamuneError::InvalidCompressionLevel { .. }))
        );
    }

    #[test]
    fn create_invalid_rootfs_compression_level_errors() {
        // ARRANGE
        let tmp = tempfile::tempdir().expect("tempdir");
        let init_file = tmp.path().join("init");
        std::fs::write(&init_file, b"init").expect("write init");
        let rootfs = tmp.path().join("rootfs");
        std::fs::create_dir_all(&rootfs).expect("mkdir rootfs");
        std::fs::write(rootfs.join("file"), b"data").expect("write");

        // ACT
        let output = tmp.path().join("initramfs.img");
        let result = create(
            &CreateConfig {
                init: &init_file,
                rootfs_dir: &rootfs,
                file_contexts: None,
                compression_level: 19,
                rootfs_compression_level: i32::MAX,
            },
            &output,
        );

        // ASSERT
        assert!(
            result
                .as_ref()
                .is_err_and(|error| matches!(error, RamuneError::InvalidCompressionLevel { .. }))
        );
    }

    #[test]
    fn prepare_rootfs_injects_required_dirs() {
        // ARRANGE
        let tmp = tempfile::tempdir().expect("tempdir");
        let rootfs = tmp.path().join("rootfs");
        std::fs::create_dir_all(&rootfs).expect("mkdir");

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
        std::fs::create_dir_all(&rootfs).expect("mkdir");

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
        let result = prepare_rootfs(&rootfs);

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
    fn copy_dir_entries_iteration_error() {
        // ARRANGE
        let src = Path::new("/virtual/src");
        let dst = Path::new("/virtual/dst");

        // ACT
        let result = copy_dir_entries(src, dst, [Err(std::io::Error::other("boom"))]);

        // ASSERT
        assert!(
            result
                .as_ref()
                .is_err_and(|error| matches!(error, RamuneError::ReadError { .. }))
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
        let result = prepare_rootfs(&rootfs);

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
        let result = prepare_rootfs(&rootfs);

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
