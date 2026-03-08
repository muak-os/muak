use std::os::unix::process::CommandExt;
use std::path::Path;
use std::process::Command;

use anyhow::{Context, Result, bail};
use rustix::mount::{MountFlags, mount, mount_move};
use rustix::process::{chdir, chroot};

pub fn switch(newroot: &str) -> Result<()> {
    move_mounts(newroot).context("move_mounts failed")?;
    bind_modules(newroot).context("bind_modules failed")?;
    chdir(newroot).context("chdir newroot failed")?;
    chroot(".").context("chroot failed")?;
    chdir("/").context("chdir / failed")?;
    delete_initramfs().context("delete_initramfs failed")?;
    exec_init().context("exec_init failed")
}

fn move_mounts(newroot: &str) -> Result<()> {
    for mnt in &["/dev", "/proc", "/sys", "/run"] {
        let target = format!("{}{}", newroot, mnt);

        std::fs::create_dir_all(&target).with_context(|| format!("Failed to create {}", target))?;

        mount_move(*mnt, target.as_str())
            .with_context(|| format!("Failed to move mount {} to {}", mnt, target))?;
    }

    Ok(())
}

/// Bind-mounts /lib/modules from initramfs into the new root
fn bind_modules(newroot: &str) -> Result<()> {
    let source = Path::new("/lib/modules");
    if !source.exists() {
        return Ok(());
    }

    let target = format!("{}/lib/modules", newroot);
    std::fs::create_dir_all(&target).with_context(|| format!("Failed to create {}", target))?;

    mount(source, target.as_str(), "", MountFlags::BIND, None)
        .with_context(|| format!("Failed to bind mount {} to {}", source.display(), target))?;

    Ok(())
}

fn delete_initramfs() -> Result<()> {
    delete_initramfs_at(Path::new("/"))
}

/// Deletes all files and directories from initramfs except special mounts.
pub fn delete_initramfs_at(root: &Path) -> Result<()> {
    let entries = std::fs::read_dir(root).context("Failed to read root directory")?;

    for entry in entries.flatten() {
        let path = entry.path();
        let name = entry.file_name();
        let name_str = name.to_string_lossy();

        if matches!(name_str.as_ref(), "dev" | "proc" | "sys" | "run" | "lib") {
            continue;
        }

        if path.is_dir() {
            let _ = std::fs::remove_dir_all(&path);
        } else {
            let _ = std::fs::remove_file(&path);
        }
    }

    Ok(())
}

fn exec_init() -> Result<()> {
    let root = Path::new("/");
    let init_path = find_init_in(root)?;
    let err = Command::new(&init_path).exec();
    bail!("Failed to exec {}: {}", init_path.display(), err);
}

/// Finds the init binary in the new root filesystem.
pub fn find_init_in(root: &Path) -> Result<std::path::PathBuf> {
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

    bail!(
        "No init binary found in new root. Checked: {:?}",
        checked_paths
    );
}

#[cfg(test)]
mod tests {
    use tempfile::TempDir;

    use super::*;

    #[test]
    fn test_delete_initramfs_preserves_special_dirs() {
        // ARRANGE
        let temp = TempDir::new().expect("Failed to create temp dir");

        std::fs::create_dir_all(temp.path().join("dev")).unwrap();
        std::fs::create_dir_all(temp.path().join("proc")).unwrap();
        std::fs::create_dir_all(temp.path().join("sys")).unwrap();
        std::fs::create_dir_all(temp.path().join("run")).unwrap();
        std::fs::create_dir_all(temp.path().join("lib")).unwrap();
        std::fs::create_dir_all(temp.path().join("old")).unwrap();
        std::fs::create_dir_all(temp.path().join("tmp")).unwrap();
        std::fs::write(temp.path().join("init"), b"#!/bin/sh").unwrap();
        std::fs::write(temp.path().join("banner"), b"Welcome").unwrap();

        // ACT
        delete_initramfs_at(temp.path()).expect("Failed to delete initramfs");

        // ASSERT
        assert!(temp.path().join("dev").exists(), "/dev should be preserved");
        assert!(
            temp.path().join("proc").exists(),
            "/proc should be preserved"
        );
        assert!(temp.path().join("sys").exists(), "/sys should be preserved");
        assert!(temp.path().join("run").exists(), "/run should be preserved");
        assert!(temp.path().join("lib").exists(), "/lib should be preserved");
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
    fn test_delete_initramfs_handles_empty_root() {
        // ARRANGE
        let temp = TempDir::new().expect("Failed to create temp dir");

        // ACT
        let result = delete_initramfs_at(temp.path());

        // ASSERT
        assert!(result.is_ok(), "Should handle empty root directory");
    }

    #[test]
    fn test_delete_initramfs_handles_only_special_dirs() {
        // ARRANGE
        let temp = TempDir::new().expect("Failed to create temp dir");

        std::fs::create_dir_all(temp.path().join("dev")).unwrap();
        std::fs::create_dir_all(temp.path().join("proc")).unwrap();

        // ACT
        let result = delete_initramfs_at(temp.path());

        // ASSERT
        assert!(
            result.is_ok(),
            "Should handle directory with only special dirs"
        );
        assert!(temp.path().join("dev").exists());
        assert!(temp.path().join("proc").exists());
    }

    #[test]
    fn test_delete_initramfs_removes_nested_structures() {
        // ARRANGE
        let temp = TempDir::new().expect("Failed to create temp dir");

        std::fs::create_dir_all(temp.path().join("old/nested/deep")).unwrap();
        std::fs::write(temp.path().join("old/file.txt"), b"content").unwrap();
        std::fs::write(temp.path().join("old/nested/another.txt"), b"content").unwrap();

        // ACT
        delete_initramfs_at(temp.path()).expect("Failed to delete initramfs");

        // ASSERT
        assert!(
            !temp.path().join("old").exists(),
            "Entire nested structure should be deleted"
        );
    }

    #[test]
    fn test_find_init_in_sbin() {
        // ARRANGE
        let temp = TempDir::new().expect("Failed to create temp dir");

        let sbin = temp.path().join("sbin");
        std::fs::create_dir_all(&sbin).unwrap();
        std::fs::write(sbin.join("init"), b"#!/bin/sh\necho init").unwrap();

        // ACT
        let result = find_init_in(temp.path());

        // ASSERT
        assert!(result.is_ok(), "Should find init in /sbin");
        assert_eq!(
            result.unwrap(),
            temp.path().join("sbin/init"),
            "Should return path to /sbin/init"
        );
    }

    #[test]
    fn test_find_init_in_bin() {
        // ARRANGE
        let temp = TempDir::new().expect("Failed to create temp dir");

        let bin = temp.path().join("bin");
        std::fs::create_dir_all(&bin).unwrap();
        std::fs::write(bin.join("init"), b"#!/bin/sh\necho init").unwrap();

        // ACT
        let result = find_init_in(temp.path());

        // ASSERT
        assert!(result.is_ok(), "Should find init in /bin");
        assert_eq!(
            result.unwrap(),
            temp.path().join("bin/init"),
            "Should return path to /bin/init"
        );
    }

    #[test]
    fn test_find_init_in_root() {
        // ARRANGE
        let temp = TempDir::new().expect("Failed to create temp dir");

        std::fs::write(temp.path().join("init"), b"#!/bin/sh\necho init").unwrap();

        // ACT
        let result = find_init_in(temp.path());

        // ASSERT
        assert!(result.is_ok(), "Should find init in root");
        assert_eq!(
            result.unwrap(),
            temp.path().join("init"),
            "Should return path to /init"
        );
    }

    #[test]
    fn test_find_init_prefers_sbin_over_bin() {
        // ARRANGE
        let temp = TempDir::new().expect("Failed to create temp dir");

        let sbin = temp.path().join("sbin");
        let bin = temp.path().join("bin");
        std::fs::create_dir_all(&sbin).unwrap();
        std::fs::create_dir_all(&bin).unwrap();
        std::fs::write(sbin.join("init"), b"#!/bin/sh\necho sbin").unwrap();
        std::fs::write(bin.join("init"), b"#!/bin/sh\necho bin").unwrap();

        // ACT
        let result = find_init_in(temp.path());

        // ASSERT
        assert!(result.is_ok());
        assert_eq!(
            result.unwrap(),
            temp.path().join("sbin/init"),
            "Should prefer /sbin/init over /bin/init"
        );
    }

    #[test]
    fn test_find_init_no_init_found() {
        // ARRANGE
        let temp = TempDir::new().expect("Failed to create temp dir");

        std::fs::create_dir_all(temp.path().join("sbin")).unwrap();
        std::fs::create_dir_all(temp.path().join("bin")).unwrap();

        // ACT
        let result = find_init_in(temp.path());

        // ASSERT
        assert!(result.is_err(), "Should fail when no init binary found");
        let err = result.unwrap_err().to_string();
        assert!(
            err.contains("No init binary found"),
            "Error should mention no init found"
        );
    }

    #[test]
    fn test_find_init_empty_root() {
        // ARRANGE
        let temp = TempDir::new().expect("Failed to create temp dir");

        // ACT
        let result = find_init_in(temp.path());

        // ASSERT
        assert!(result.is_err(), "Should fail with empty root");
    }
}
