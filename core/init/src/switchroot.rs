//! Switch the running root from the initramfs ramfs to the EROFS-backed root.

use std::os::unix::process::CommandExt as _;
use std::path::Path;
use std::process::Command;

use anyhow::{Context as _, Result, bail};
use rustix::mount::mount_move;
use rustix::process::{chdir, chroot};

/// Switches to the new root filesystem and executes the init process.
pub fn switch_root(newroot: &str) -> Result<()> {
    move_pseudo_mounts(newroot).context("moving pseudo mounts failed")?;
    chdir(newroot).context("chdir newroot failed")?;
    chroot(".").context("chroot failed")?;
    chdir("/").context("chdir / failed")?;
    free_ramfs_at(Path::new("/")).context("free ramfs failed")?;
    exec_init().context("exec init failed")
}

/// Moves pseudo-filesystem mounts from the initramfs into the new root.
fn move_pseudo_mounts(newroot: &str) -> Result<()> {
    for mnt in &["/dev", "/proc", "/sys", "/run"] {
        let target = format!("{newroot}{mnt}");

        mount_move(*mnt, target.as_str())
            .with_context(|| format!("Failed to move mount {mnt} to {target}"))?;
    }

    Ok(())
}

/// Frees RAM by deleting all initramfs contents from the old root, preserving moved mounts.
fn free_ramfs_at(root: &Path) -> Result<()> {
    let entries = std::fs::read_dir(root).context("Failed to read root directory")?;

    for entry in entries.flatten() {
        let path = entry.path();
        let name = entry.file_name();
        let name_str = name.to_string_lossy();

        if matches!(name_str.as_ref(), "dev" | "proc" | "sys" | "run") {
            continue;
        }

        if path.is_dir() {
            drop(std::fs::remove_dir_all(&path));
        } else {
            drop(std::fs::remove_file(&path));
        }
    }

    Ok(())
}

/// Executes the init binary from the new root filesystem.
fn exec_init() -> Result<()> {
    let root = Path::new("/");
    let init_path = find_init(root)?;
    let err = Command::new(&init_path).exec();
    bail!("Failed to exec {}: {}", init_path.display(), err);
}

/// Finds the init binary in the new root filesystem.
fn find_init(root: &Path) -> Result<std::path::PathBuf> {
    let init_paths = ["sbin/init", "bin/init", "init"];

    let mut checked_paths = Vec::new();
    for init_path in &init_paths {
        let full_path = root.join(init_path);
        let exists = full_path.exists();
        checked_paths.push(format!("{} exists={}", full_path.display(), exists));

        if exists {
            return Ok(full_path);
        }
    }

    bail!("No init binary found in new root. Checked: {checked_paths:?}");
}

#[cfg(test)]
mod tests {
    use tempfile::TempDir;

    use super::*;

    #[test]
    fn free_ramfs_preserves_pseudo_mounts() {
        // ARRANGE
        let temp = TempDir::new().expect("Failed to create temp dir");

        std::fs::create_dir_all(temp.path().join("dev")).unwrap();
        std::fs::create_dir_all(temp.path().join("proc")).unwrap();
        std::fs::create_dir_all(temp.path().join("sys")).unwrap();
        std::fs::create_dir_all(temp.path().join("run")).unwrap();
        std::fs::create_dir_all(temp.path().join("old")).unwrap();
        std::fs::create_dir_all(temp.path().join("tmp")).unwrap();
        std::fs::write(temp.path().join("init"), b"#!/bin/sh").unwrap();
        std::fs::write(temp.path().join("banner"), b"Welcome").unwrap();

        // ACT
        free_ramfs_at(temp.path()).expect("Failed to free ramfs");

        // ASSERT
        assert!(temp.path().join("dev").exists(), "/dev should be preserved");
        assert!(
            temp.path().join("proc").exists(),
            "/proc should be preserved"
        );
        assert!(temp.path().join("sys").exists(), "/sys should be preserved");
        assert!(temp.path().join("run").exists(), "/run should be preserved");
        assert!(
            !temp.path().join("old").exists(),
            "Regular directory /old should be deleted"
        );
        assert!(
            !temp.path().join("tmp").exists(),
            "Regular directory /tmp should be deleted"
        );
        assert!(
            !temp.path().join("init").exists(),
            "Regular file /init should be deleted"
        );
        assert!(
            !temp.path().join("banner").exists(),
            "Regular file /banner should be deleted"
        );
    }

    #[test]
    fn free_ramfs_handles_empty_root() {
        // ARRANGE
        let temp = TempDir::new().expect("Failed to create temp dir");

        // ACT
        let result = free_ramfs_at(temp.path());

        // ASSERT
        assert!(result.is_ok(), "Should handle empty root directory");
    }

    #[test]
    fn free_ramfs_handles_only_pseudo_mounts() {
        // ARRANGE
        let temp = TempDir::new().expect("Failed to create temp dir");

        std::fs::create_dir_all(temp.path().join("dev")).unwrap();
        std::fs::create_dir_all(temp.path().join("proc")).unwrap();

        // ACT
        let result = free_ramfs_at(temp.path());

        // ASSERT
        assert!(
            result.is_ok(),
            "Should handle directory with only pseudo mount dirs"
        );
        assert!(temp.path().join("dev").exists());
        assert!(temp.path().join("proc").exists());
    }

    #[test]
    fn free_ramfs_removes_nested_structures() {
        // ARRANGE
        let temp = TempDir::new().expect("Failed to create temp dir");

        std::fs::create_dir_all(temp.path().join("old/nested/deep")).unwrap();
        std::fs::write(temp.path().join("old/file.txt"), b"content").unwrap();
        std::fs::write(temp.path().join("old/nested/another.txt"), b"content").unwrap();

        // ACT
        free_ramfs_at(temp.path()).expect("Failed to free ramfs");

        // ASSERT
        assert!(
            !temp.path().join("old").exists(),
            "Entire nested structure should be deleted"
        );
    }

    #[test]
    fn find_init_in_sbin() {
        // ARRANGE
        let temp = TempDir::new().expect("Failed to create temp dir");

        let sbin = temp.path().join("sbin");
        std::fs::create_dir_all(&sbin).unwrap();
        std::fs::write(sbin.join("init"), b"#!/bin/sh\necho init").unwrap();

        // ACT
        let result = find_init(temp.path());

        // ASSERT
        assert!(result.is_ok(), "Should find init in /sbin");
        assert_eq!(
            result.unwrap(),
            temp.path().join("sbin/init"),
            "Should return path to /sbin/init"
        );
    }

    #[test]
    fn find_init_in_bin() {
        // ARRANGE
        let temp = TempDir::new().expect("Failed to create temp dir");

        let bin = temp.path().join("bin");
        std::fs::create_dir_all(&bin).unwrap();
        std::fs::write(bin.join("init"), b"#!/bin/sh\necho init").unwrap();

        // ACT
        let result = find_init(temp.path());

        // ASSERT
        assert!(result.is_ok(), "Should find init in /bin");
        assert_eq!(
            result.unwrap(),
            temp.path().join("bin/init"),
            "Should return path to /bin/init"
        );
    }

    #[test]
    fn find_init_in_root() {
        // ARRANGE
        let temp = TempDir::new().expect("Failed to create temp dir");

        std::fs::write(temp.path().join("init"), b"#!/bin/sh\necho init").unwrap();

        // ACT
        let result = find_init(temp.path());

        // ASSERT
        assert!(result.is_ok(), "Should find init in root");
        assert_eq!(
            result.unwrap(),
            temp.path().join("init"),
            "Should return path to /init"
        );
    }

    #[test]
    fn find_init_prefers_sbin_over_bin() {
        // ARRANGE
        let temp = TempDir::new().expect("Failed to create temp dir");

        let sbin = temp.path().join("sbin");
        let bin = temp.path().join("bin");
        std::fs::create_dir_all(&sbin).unwrap();
        std::fs::create_dir_all(&bin).unwrap();
        std::fs::write(sbin.join("init"), b"#!/bin/sh\necho sbin").unwrap();
        std::fs::write(bin.join("init"), b"#!/bin/sh\necho bin").unwrap();

        // ACT
        let result = find_init(temp.path());

        // ASSERT
        assert!(result.is_ok());
        assert_eq!(
            result.unwrap(),
            temp.path().join("sbin/init"),
            "Should prefer /sbin/init over /bin/init"
        );
    }

    #[test]
    fn find_init_no_init_found() {
        // ARRANGE
        let temp = TempDir::new().expect("Failed to create temp dir");

        std::fs::create_dir_all(temp.path().join("sbin")).unwrap();
        std::fs::create_dir_all(temp.path().join("bin")).unwrap();

        // ACT
        let result = find_init(temp.path());

        // ASSERT
        assert!(result.is_err(), "Should fail when no init binary found");
        let err = result.unwrap_err().to_string();
        assert!(
            err.contains("No init binary found"),
            "Error should mention no init found"
        );
    }

    #[test]
    fn find_init_empty_root() {
        // ARRANGE
        let temp = TempDir::new().expect("Failed to create temp dir");

        // ACT
        let result = find_init(temp.path());

        // ASSERT
        assert!(result.is_err(), "Should fail with empty root");
    }
}
