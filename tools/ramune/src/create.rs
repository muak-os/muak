//! Base initramfs creation from init binary, rootfs directory, and kernel modules.

use std::collections::BTreeMap;
use std::path::Path;

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

/// Configuration for base initramfs creation.
pub struct CreateConfig<'a> {
    pub init: &'a Path,
    pub rootfs_dir: &'a Path,
    pub modules: &'a Path,
    pub file_contexts: Option<&'a ::erofs::FileContexts>,
}

/// Creates a base initramfs image from an init binary, rootfs directory, and kernel modules.
pub(crate) fn create_initramfs(config: &CreateConfig<'_>) -> Result<Vec<u8>> {
    let init_data = std::fs::read(config.init).map_err(|e| RamuneError::ReadError {
        file: config.init.display().to_string(),
        source: e,
    })?;

    let rootfs_erofs = erofs::create(config.rootfs_dir, config.file_contexts)?;

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
    walk_dir(modules_dir, modules_dir, &mut tree)?;

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
fn walk_dir(base: &Path, dir: &Path, tree: &mut BTreeMap<String, Vec<u8>>) -> Result<()> {
    let read_dir = std::fs::read_dir(dir).map_err(|e| RamuneError::ReadError {
        file: dir.display().to_string(),
        source: e,
    })?;

    for entry in read_dir {
        let entry = entry.map_err(|e| RamuneError::ReadError {
            file: dir.display().to_string(),
            source: e,
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

        if path.is_dir() {
            walk_dir(base, &path, tree)?;
        } else {
            let data = std::fs::read(&path).map_err(|e| RamuneError::ReadError {
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

        let config = CreateConfig {
            init: &init_file,
            rootfs_dir: &rootfs,
            modules: &modules,
            file_contexts: None,
        };

        // ACT
        let result = create_initramfs(&config).expect("create_initramfs");

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

        let config = CreateConfig {
            init: &init_file,
            rootfs_dir: &rootfs,
            modules: &modules,
            file_contexts: None,
        };

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

        let config = CreateConfig {
            init: Path::new("/nonexistent/init"),
            rootfs_dir: &rootfs,
            modules: &modules,
            file_contexts: None,
        };

        // ACT
        let result = create_initramfs(&config);

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

        let config = CreateConfig {
            init: &init_file,
            rootfs_dir: &rootfs,
            modules: &modules,
            file_contexts: Some(&fc),
        };

        // ACT
        let result = create_initramfs(&config).expect("create_initramfs");

        // ASSERT
        assert!(!result.is_empty());
    }
}
