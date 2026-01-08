use anyhow::{Result, bail};
use std::path::PathBuf;
use std::process::Command;

use super::DATA_DIR;

pub fn create_subvolume(vm_id: &str) -> Result<PathBuf> {
    let path = PathBuf::from(DATA_DIR).join(vm_id);

    let output = Command::new("/sbin/btrfs")
        .args(["subvolume", "create"])
        .arg(&path)
        .output()?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        bail!("Failed to create subvolume {}: {}", path.display(), stderr);
    }

    kmsg::info!(@ "vmd", "Created btrfs subvolume at {}", path.display());
    Ok(path)
}

pub fn delete_subvolume(vm_id: &str) -> Result<()> {
    let path = PathBuf::from(DATA_DIR).join(vm_id);

    if !path.exists() {
        return Ok(());
    }

    let output = Command::new("/sbin/btrfs")
        .args(["subvolume", "delete"])
        .arg(&path)
        .output()?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        bail!("Failed to delete subvolume {}: {}", path.display(), stderr);
    }

    kmsg::info!(@ "vmd", "Deleted btrfs subvolume at {}", path.display());
    Ok(())
}

pub fn list_subvolumes() -> Result<Vec<String>> {
    let output = Command::new("/sbin/btrfs")
        .args(["subvolume", "list", "-o", DATA_DIR])
        .output()?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        bail!("Failed to list subvolumes: {}", stderr);
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let mut vm_ids = Vec::new();

    for line in stdout.lines() {
        if let Some(path_part) = line.split(" path ").nth(1) {
            if let Some(vm_id) =
                path_part.strip_prefix(&format!("{}/", DATA_DIR.trim_start_matches('/')))
            {
                if !vm_id.contains('/') {
                    vm_ids.push(vm_id.to_string());
                }
            } else if let Some(vm_id) = path_part.rsplit('/').next()
                && !vm_id.is_empty()
            {
                vm_ids.push(vm_id.to_string());
            }
        }
    }

    Ok(vm_ids)
}
