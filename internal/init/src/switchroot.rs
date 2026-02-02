use std::fs;
use std::os::unix::process::CommandExt;
use std::path::Path;
use std::process::Command;

use anyhow::{Context, Result, bail};
use rustix::mount::mount_move;
use rustix::process::{chdir, chroot};

pub fn switch(newroot: &str) -> Result<()> {
    move_mounts(newroot).context("move_mounts failed")?;
    chdir(newroot).context("chdir newroot failed")?;
    chroot(".").context("chroot failed")?;
    chdir("/").context("chdir / failed")?;
    delete_initramfs().context("delete_initramfs failed")?;
    exec_init().context("exec_init failed")?;

    unreachable!("exec_init should never return");
}

fn move_mounts(newroot: &str) -> Result<()> {
    for mnt in &["/dev", "/proc", "/sys", "/run"] {
        let target = format!("{}{}", newroot, mnt);

        fs::create_dir_all(&target).with_context(|| format!("Failed to create {}", target))?;

        mount_move(*mnt, target.as_str())
            .with_context(|| format!("Failed to move mount {} to {}", mnt, target))?;
    }

    Ok(())
}

fn delete_initramfs() -> Result<()> {
    delete_initramfs_at(Path::new("/"))
}

/// Deletes all files and directories from initramfs except special mounts.
pub fn delete_initramfs_at(root: &Path) -> Result<()> {
    let entries = fs::read_dir(root).context("Failed to read root directory")?;

    for entry in entries.flatten() {
        let path = entry.path();
        let name = entry.file_name();
        let name_str = name.to_string_lossy();

        if name_str == "dev" || name_str == "proc" || name_str == "sys" || name_str == "run" {
            continue;
        }

        if path.is_dir() {
            let _ = fs::remove_dir_all(&path);
        } else {
            let _ = fs::remove_file(&path);
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
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_delete_initramfs_preserves_special_dirs() {
        let temp = TempDir::new().expect("Failed to create temp dir");

        fs::create_dir_all(temp.path().join("dev")).unwrap();
        fs::create_dir_all(temp.path().join("proc")).unwrap();
        fs::create_dir_all(temp.path().join("sys")).unwrap();
        fs::create_dir_all(temp.path().join("run")).unwrap();
        fs::create_dir_all(temp.path().join("old")).unwrap();
        fs::create_dir_all(temp.path().join("tmp")).unwrap();
        fs::write(temp.path().join("init"), b"#!/bin/sh").unwrap();
        fs::write(temp.path().join("banner"), b"Welcome").unwrap();

        delete_initramfs_at(temp.path()).expect("Failed to delete initramfs");

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
    fn test_delete_initramfs_handles_empty_root() {
        let temp = TempDir::new().expect("Failed to create temp dir");

        let result = delete_initramfs_at(temp.path());

        assert!(result.is_ok(), "Should handle empty root directory");
    }

    #[test]
    fn test_delete_initramfs_handles_only_special_dirs() {
        let temp = TempDir::new().expect("Failed to create temp dir");

        fs::create_dir_all(temp.path().join("dev")).unwrap();
        fs::create_dir_all(temp.path().join("proc")).unwrap();

        let result = delete_initramfs_at(temp.path());

        assert!(
            result.is_ok(),
            "Should handle directory with only special dirs"
        );
        assert!(temp.path().join("dev").exists());
        assert!(temp.path().join("proc").exists());
    }

    #[test]
    fn test_delete_initramfs_removes_nested_structures() {
        let temp = TempDir::new().expect("Failed to create temp dir");

        fs::create_dir_all(temp.path().join("old/nested/deep")).unwrap();
        fs::write(temp.path().join("old/file.txt"), b"content").unwrap();
        fs::write(temp.path().join("old/nested/another.txt"), b"content").unwrap();

        delete_initramfs_at(temp.path()).expect("Failed to delete initramfs");

        assert!(
            !temp.path().join("old").exists(),
            "Entire nested structure should be deleted"
        );
    }

    #[test]
    fn test_find_init_in_sbin() {
        let temp = TempDir::new().expect("Failed to create temp dir");

        let sbin = temp.path().join("sbin");
        fs::create_dir_all(&sbin).unwrap();
        fs::write(sbin.join("init"), b"#!/bin/sh\necho init").unwrap();

        let result = find_init_in(temp.path());

        assert!(result.is_ok(), "Should find init in /sbin");
        assert_eq!(
            result.unwrap(),
            temp.path().join("sbin/init"),
            "Should return path to /sbin/init"
        );
    }

    #[test]
    fn test_find_init_in_bin() {
        let temp = TempDir::new().expect("Failed to create temp dir");

        let bin = temp.path().join("bin");
        fs::create_dir_all(&bin).unwrap();
        fs::write(bin.join("init"), b"#!/bin/sh\necho init").unwrap();

        let result = find_init_in(temp.path());

        assert!(result.is_ok(), "Should find init in /bin");
        assert_eq!(
            result.unwrap(),
            temp.path().join("bin/init"),
            "Should return path to /bin/init"
        );
    }

    #[test]
    fn test_find_init_in_root() {
        let temp = TempDir::new().expect("Failed to create temp dir");

        fs::write(temp.path().join("init"), b"#!/bin/sh\necho init").unwrap();

        let result = find_init_in(temp.path());

        assert!(result.is_ok(), "Should find init in root");
        assert_eq!(
            result.unwrap(),
            temp.path().join("init"),
            "Should return path to /init"
        );
    }

    #[test]
    fn test_find_init_prefers_sbin_over_bin() {
        let temp = TempDir::new().expect("Failed to create temp dir");

        let sbin = temp.path().join("sbin");
        let bin = temp.path().join("bin");
        fs::create_dir_all(&sbin).unwrap();
        fs::create_dir_all(&bin).unwrap();
        fs::write(sbin.join("init"), b"#!/bin/sh\necho sbin").unwrap();
        fs::write(bin.join("init"), b"#!/bin/sh\necho bin").unwrap();

        let result = find_init_in(temp.path());

        assert!(result.is_ok());
        assert_eq!(
            result.unwrap(),
            temp.path().join("sbin/init"),
            "Should prefer /sbin/init over /bin/init"
        );
    }

    #[test]
    fn test_find_init_no_init_found() {
        let temp = TempDir::new().expect("Failed to create temp dir");

        fs::create_dir_all(temp.path().join("sbin")).unwrap();
        fs::create_dir_all(temp.path().join("bin")).unwrap();

        let result = find_init_in(temp.path());

        assert!(result.is_err(), "Should fail when no init binary found");
        let err = result.unwrap_err().to_string();
        assert!(
            err.contains("No init binary found"),
            "Error should mention no init found"
        );
    }

    #[test]
    fn test_find_init_empty_root() {
        let temp = TempDir::new().expect("Failed to create temp dir");

        let result = find_init_in(temp.path());

        assert!(result.is_err(), "Should fail with empty root");
    }
}
