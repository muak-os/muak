use std::fs;
use std::path::Path;

use anyhow::{Context, Result, bail};

/// File in the SELinux filesystem where the policy blob should be written to load it into the kernel.
const SELINUXFS_LOAD: &str = "/sys/fs/selinux/load";

/// File in the SELinux filesystem that indicates whether SELinux is enforcing or permissive.
const SELINUXFS_ENFORCE: &str = "/sys/fs/selinux/enforce";

/// Directory where SELinux policy files are expected to be found.
const POLICY_DIR: &str = "/newroot/etc/selinux";

/// Finds, reads, and loads the SELinux policy into the kernel.
pub fn load() -> Result<()> {
    let path = fs::read_dir(POLICY_DIR)
        .with_context(|| format!("Failed to read {}", POLICY_DIR))?
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .find(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.starts_with("policy."))
        })
        .with_context(|| format!("No policy.* file found in {}", POLICY_DIR))?;

    if !Path::new(SELINUXFS_LOAD).exists() {
        bail!("SELinux filesystem is not mounted at {}", SELINUXFS_LOAD);
    }

    let policy_bytes = fs::read(&path)
        .with_context(|| format!("Failed to read SELinux policy from {}", path.display()))?;
    fs::write(SELINUXFS_LOAD, policy_bytes).context("Failed to write SELinux policy blob to kernel")
}

/// Returns true if SELinux is in enforcing mode, false if permissive.
pub fn is_enforcing() -> Result<bool> {
    let val = fs::read_to_string(SELINUXFS_ENFORCE).context("Failed to read SELinux enforce")?;
    Ok(val.trim() == "1")
}
