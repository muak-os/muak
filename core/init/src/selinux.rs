use std::fs;
use std::path::Path;

use anyhow::{Context, Result, bail};

const SELINUXFS_LOAD: &str = "/sys/fs/selinux/load";
const SELINUXFS_ENFORCE: &str = "/sys/fs/selinux/enforce";

/// Load a compiled SELinux binary policy blob into the kernel.
pub fn load_policy(policy_bytes: &[u8]) -> Result<()> {
    if !Path::new(SELINUXFS_LOAD).exists() {
        bail!("SELinux filesystem is not mounted at {}", SELINUXFS_LOAD);
    }
    fs::write(SELINUXFS_LOAD, policy_bytes).context("Failed to write SELinux policy blob to kernel")
}

/// Returns true if SELinux is in enforcing mode, false if permissive.
pub fn is_enforcing() -> Result<bool> {
    let val = fs::read_to_string(SELINUXFS_ENFORCE).context("Failed to read SELinux enforce")?;
    Ok(val.trim() == "1")
}
