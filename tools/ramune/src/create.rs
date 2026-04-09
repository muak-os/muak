//! Base initramfs creation from init binary, rootfs directory, and kernel modules.

use std::collections::BTreeMap;
use std::os::unix::fs as unix_fs;
use std::path::Path;

use walkdir::WalkDir;

use crate::cpio::{self, CpioEntry};
use crate::erofs;
use crate::error::{RamuneError, Result};

/// Zstd compression level for the initramfs archive.
const COMPRESSION_LEVEL: i32 = 19;

/// CPIO mode for regular executable files.
const MODE_EXEC: u32 = 0o100755;

/// CPIO mode for regular files.
const MODE_FILE: u32 = 0o100644;

/// CPIO mode for directories.
const MODE_DIR: u32 = 0o040755;

/// Directories that must always exist in the rootfs.
const REQUIRED_DIRS: &[&str] = &["dev", "proc", "sys", "run", "etc/services", "etc/selinux"];

/// Configuration for base initramfs creation.
pub struct CreateConfig<'a> {
    pub init: &'a Path,
    pub rootfs_dir: &'a Path,
    pub modules: &'a Path,
    pub file_contexts: Option<&'a ::erofs::FileContexts>,
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

/// Creates a base initramfs image from an init binary, rootfs directory, and kernel modules.
pub(crate) fn create_initramfs(config: &CreateConfig<'_>) -> Result<Vec<u8>> {
    let init_data = std::fs::read(config.init).map_err(|e| RamuneError::ReadError {
        file: config.init.display().to_string(),
        source: e,
    })?;

    let staging = prepare_rootfs(config.rootfs_dir)?;
    let rootfs_erofs = erofs::create(staging.path(), config.file_contexts)?;

    let mut entries = vec![
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

    collect_modules(config.modules, &mut entries)?;

    let cpio_data = cpio::create_from_entries(&entries)?;
    zstd::encode_all(&cpio_data[..], COMPRESSION_LEVEL)
        .map_err(|e| RamuneError::CpioError(format!("Compression failed: {e}")))
}

/// Recursively collects files from a directory into sorted CPIO entries with `lib/modules` prefix.
fn collect_modules(modules_dir: &Path, entries: &mut Vec<CpioEntry>) -> Result<()> {
    let mut tree: BTreeMap<String, Vec<u8>> = BTreeMap::new();
    walk_dir(modules_dir, &mut tree)?;

    let mut dirs_emitted = std::collections::BTreeSet::new();
    dirs_emitted.insert("lib".to_string());
    dirs_emitted.insert("lib/modules".to_string());

    entries.push(CpioEntry {
        path: "lib".to_string(),
        mode: MODE_DIR,
        data: Vec::new(),
    });
    entries.push(CpioEntry {
        path: "lib/modules".to_string(),
        mode: MODE_DIR,
        data: Vec::new(),
    });

    for (rel_path, data) in &tree {
        let full_path = format!("lib/modules/{rel_path}");
        emit_parent_dirs(&full_path, &mut dirs_emitted, entries);
        entries.push(CpioEntry {
            path: full_path,
            mode: MODE_FILE,
            data: data.clone(),
        });
    }

    Ok(())
}

/// Emits directory entries for all ancestors of `path` not yet in `emitted`.
fn emit_parent_dirs(
    path: &str,
    emitted: &mut std::collections::BTreeSet<String>,
    entries: &mut Vec<CpioEntry>,
) {
    let parts: Vec<&str> = path.split('/').collect();
    for i in 1..parts.len() {
        let dir = parts[..i].join("/");
        if emitted.insert(dir.clone()) {
            entries.push(CpioEntry {
                path: dir,
                mode: MODE_DIR,
                data: Vec::new(),
            });
        }
    }
}

/// Recursively walks a directory, collecting relative paths and file contents.
fn walk_dir(base: &Path, tree: &mut BTreeMap<String, Vec<u8>>) -> Result<()> {
    for entry in WalkDir::new(base).min_depth(1) {
        let entry = entry.map_err(|e| RamuneError::ReadError {
            file: base.display().to_string(),
            source: std::io::Error::other(e),
        })?;
        let path = entry.path();
        let rel = path
            .strip_prefix(base)
            .map_err(|e| RamuneError::ReadError {
                file: path.display().to_string(),
                source: std::io::Error::other(e),
            })?
            .to_string_lossy()
            .into_owned();

        if entry.file_type().is_file() {
            let data = std::fs::read(path).map_err(|e| RamuneError::ReadError {
                file: path.display().to_string(),
                source: e,
            })?;
            tree.insert(rel, data);
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn setup_modules_dir(dir: &Path) {
        let v = dir.join("6.19.8");
        std::fs::create_dir_all(v.join("kernel/drivers")).expect("mkdir");
        std::fs::write(v.join("kernel/drivers/test.ko"), b"module").expect("write");
        std::fs::write(v.join("modules.dep"), b"").expect("write");
    }

    fn setup_rootfs_dir(dir: &Path) {
        std::fs::create_dir_all(dir.join("sbin")).expect("mkdir");
        std::fs::write(dir.join("sbin/init"), b"init-binary").expect("write");
    }

    fn make_config<'a>(init: &'a Path, rootfs: &'a Path, modules: &'a Path) -> CreateConfig<'a> {
        CreateConfig {
            init,
            rootfs_dir: rootfs,
            modules,
            file_contexts: None,
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
        let modules = tmp.path().join("modules");
        std::fs::create_dir(&modules).expect("mkdir modules");
        setup_modules_dir(&modules);

        // ACT
        let result = create_initramfs(&make_config(&init_file, &rootfs, &modules))
            .expect("create_initramfs");

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
        let modules = tmp.path().join("modules");
        std::fs::create_dir_all(modules.join("6.19.8")).expect("mkdir");
        std::fs::write(modules.join("6.19.8/mod.ko"), b"ko").expect("write");
        let config = make_config(&init_file, &rootfs, &modules);

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
        let modules = tmp.path().join("modules");
        std::fs::create_dir(&modules).expect("mkdir");

        // ACT
        let result = create_initramfs(&CreateConfig {
            init: Path::new("/nonexistent/init"),
            rootfs_dir: &rootfs,
            modules: &modules,
            file_contexts: None,
        });

        // ASSERT
        assert!(matches!(result, Err(RamuneError::ReadError { .. })));
    }

    #[test]
    fn collect_modules_nested_structure() {
        // ARRANGE
        let tmp = tempfile::tempdir().expect("tempdir");
        setup_modules_dir(tmp.path());
        let mut entries = Vec::new();

        // ACT
        collect_modules(tmp.path(), &mut entries).expect("collect_modules");

        // ASSERT
        let paths: Vec<&str> = entries.iter().map(|e| e.path.as_str()).collect();
        assert!(paths.contains(&"lib"));
        assert!(paths.contains(&"lib/modules"));
        assert!(paths.contains(&"lib/modules/6.19.8"));
        assert!(paths.contains(&"lib/modules/6.19.8/kernel"));
        assert!(paths.contains(&"lib/modules/6.19.8/kernel/drivers"));
        assert!(paths.contains(&"lib/modules/6.19.8/kernel/drivers/test.ko"));
        assert!(paths.contains(&"lib/modules/6.19.8/modules.dep"));
    }

    #[test]
    fn collect_modules_empty_dir() {
        // ARRANGE
        let tmp = tempfile::tempdir().expect("tempdir");
        let mut entries = Vec::new();

        // ACT
        collect_modules(tmp.path(), &mut entries).expect("collect_modules");

        // ASSERT
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].path, "lib");
        assert_eq!(entries[1].path, "lib/modules");
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
        let modules = tmp.path().join("modules");
        std::fs::create_dir(&modules).expect("mkdir");
        let fc =
            ::erofs::FileContexts::from_reader("/.*    system_u:object_r:file_t:s0\n".as_bytes())
                .expect("fc");

        // ACT
        let result = create_initramfs(&CreateConfig {
            init: &init_file,
            rootfs_dir: &rootfs,
            modules: &modules,
            file_contexts: Some(&fc),
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
