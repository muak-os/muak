use anyhow::{Result, bail};
use std::path::PathBuf;
use std::process::Command;

use super::DATA_DIR;

#[derive(Debug, Clone, Default)]
pub struct DiskUsage {
    pub used_bytes: u64,
    pub quota_bytes: u64,
    pub usage_percent: f32,
}

pub fn set_quota(vm_id: &str, size_bytes: u64) -> Result<()> {
    let path = PathBuf::from(DATA_DIR).join(vm_id);

    let output = Command::new("/sbin/btrfs")
        .args(["qgroup", "limit", &size_bytes.to_string()])
        .arg(&path)
        .output()?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        bail!("Failed to set quota on {}: {}", path.display(), stderr);
    }

    kmsg::info!(@ "vmd", "Set quota {} bytes on {}", size_bytes, path.display());
    Ok(())
}

pub fn get_usage(vm_id: &str) -> Result<DiskUsage> {
    let path = PathBuf::from(DATA_DIR).join(vm_id);

    let output = Command::new("/sbin/btrfs")
        .args(["qgroup", "show", "-reF", "--raw"])
        .arg(&path)
        .output()?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        bail!(
            "Failed to get quota usage for {}: {}",
            path.display(),
            stderr
        );
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    parse_qgroup_output(&stdout)
}

fn parse_qgroup_output(output: &str) -> Result<DiskUsage> {
    for line in output.lines().skip(2) {
        let parts: Vec<&str> = line.split_whitespace().collect();
        if parts.len() >= 4 {
            let used_bytes = parts[1].parse::<u64>().unwrap_or(0);
            let quota_bytes = if parts[3] == "none" {
                0
            } else {
                parts[3].parse::<u64>().unwrap_or(0)
            };

            let usage_percent = if quota_bytes > 0 {
                (used_bytes as f64 / quota_bytes as f64 * 100.0) as f32
            } else {
                0.0
            };

            return Ok(DiskUsage {
                used_bytes,
                quota_bytes,
                usage_percent,
            });
        }
    }

    Ok(DiskUsage::default())
}
