use nix::mount::{MsFlags, mount};
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
    for mnt in &["/dev", "/proc", "/sys", "/mnt"] {
        let target = format!("{}{}", newroot, mnt);

        fs::create_dir_all(&target).map_err(|e| format!("Failed to create {}: {}", target, e))?;

        mount(
            Some(*mnt),
            target.as_str(),
            None::<&str>,
            MsFlags::MS_MOVE,
            None::<&str>,
        )
        .map_err(|e| format!("Failed to move mount {} to {}: {}", mnt, target, e))?;
    }

    // Create /run as tmpfs in new root and copy over the contents
    let run_target = format!("{}/run", newroot);
    fs::create_dir_all(&run_target)
        .map_err(|e| format!("Failed to create /run in new root: {}", e))?;

    mount(
        Some("tmpfs"),
        run_target.as_str(),
        Some("tmpfs"),
        MsFlags::empty(),
        Some("mode=0755"),
    )
    .map_err(|e| format!("Failed to mount tmpfs on /run in new root: {}", e))?;

    // Copy /run/uki contents from initramfs to new root
    if let Ok(entries) = fs::read_dir("/run/uki") {
        let uki_target = format!("{}/run/uki", newroot);
        fs::create_dir_all(&uki_target)
            .map_err(|e| format!("Failed to create /run/uki in new root: {}", e))?;

        for entry in entries.flatten() {
            let src_path = entry.path();
            let filename = entry.file_name();
            let dst_path = format!("{}/{}", uki_target, filename.to_string_lossy());

            if src_path.is_file() {
                fs::copy(&src_path, &dst_path).map_err(|e| {
                    format!(
                        "Failed to copy {} to {}: {}",
                        src_path.display(),
                        dst_path,
                        e
                    )
                })?;
            }
        }
    }

    Ok(())
}

fn delete_initramfs() -> Result<(), Box<dyn std::error::Error>> {
    let entries = fs::read_dir("/")?;

    for entry in entries.flatten() {
        let path = entry.path();
        let name = entry.file_name();
        let name_str = name.to_string_lossy();

        if name_str == "dev"
            || name_str == "proc"
            || name_str == "sys"
            || name_str == "run"
            || name_str == "mnt"
        {
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

fn exec_init() -> Result<(), Box<dyn std::error::Error>> {
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
            return Err(format!("Failed to exec {}: {}", init_path, err).into());
        }
    }

    Err(format!(
        "No init binary found in new root. Checked: {:?}",
        checked_paths
    )
    .into())
}
