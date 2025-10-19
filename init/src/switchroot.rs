use nix::mount::{mount, MsFlags};
use nix::unistd::{chdir, chroot};
use std::fs;
use std::os::unix::process::CommandExt;
use std::process::Command;

pub fn switch(newroot: &str) -> Result<(), Box<dyn std::error::Error>> {
    move_mounts(newroot).map_err(|e| format!("move_mounts failed: {}", e))?;
    chdir(newroot).map_err(|e| format!("chdir newroot failed: {}", e))?;
    chroot(".").map_err(|e| format!("chroot failed: {}", e))?;
    chdir("/").map_err(|e| format!("chdir / failed: {}", e))?;
    delete_initramfs().map_err(|e| format!("delete_initramfs failed: {}", e))?;
    exec_init().map_err(|e| format!("exec_init failed: {}", e))?;

    unreachable!("exec_init should never return");
}

fn move_mounts(newroot: &str) -> Result<(), Box<dyn std::error::Error>> {
    for mnt in &["/dev", "/proc", "/sys", "/run"] {
        let target = format!("{}{}", newroot, mnt);

        if let Err(_) = fs::create_dir_all(&target) {}

        mount(
            Some(*mnt),
            target.as_str(),
            None::<&str>,
            MsFlags::MS_MOVE,
            None::<&str>,
        )?;
    }

    Ok(())
}

fn delete_initramfs() -> Result<(), Box<dyn std::error::Error>> {
    let entries = fs::read_dir("/")?;

    for entry in entries {
        if let Ok(entry) = entry {
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
    }

    Ok(())
}

fn exec_init() -> Result<(), Box<dyn std::error::Error>> {
    let init_paths = ["/sbin/init", "/bin/init", "/init"];

    let mut checked_paths = Vec::new();
    for init_path in &init_paths {
        checked_paths.push(format!("{} exists={}", init_path, fs::metadata(init_path).is_ok()));
        if fs::metadata(init_path).is_ok() {
            let err = Command::new(init_path).exec();
            return Err(format!("Failed to exec {}: {}", init_path, err).into());
        }
    }

    Err(format!("No init binary found in new root. Checked: {:?}", checked_paths).into())
}
