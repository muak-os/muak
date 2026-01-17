use anyhow::{Context, Result, bail};
use rustix::mount::mount_move;
use rustix::process::{chdir, chroot};
use std::fs;
use std::os::unix::process::CommandExt;
use std::process::Command;

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
    let entries = fs::read_dir("/").context("Failed to read /")?;

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
    let init_paths = ["/sbin/init", "/bin/init", "/init"];

    let mut checked_paths = Vec::new();
    for init_path in &init_paths {
        checked_paths.push(format!(
            "{} exists={}",
            init_path,
            fs::metadata(init_path).is_ok()
        ));
        if fs::metadata(init_path).is_ok() {
            let err = Command::new(init_path).exec();
            bail!("Failed to exec {}: {}", init_path, err);
        }
    }

    bail!(
        "No init binary found in new root. Checked: {:?}",
        checked_paths
    );
}
