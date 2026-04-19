//! Sysctl execution for early-boot kernel configuration.

use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};

/// Procfs directory containing sysctl files.
const PROC_SYS_DIR: &str = "/proc/sys";

/// Embedded architecture-specific early-boot sysctl policy.
#[cfg(target_arch = "x86_64")]
const SYSCTL_CONF: &str = include_str!("../../kernel/sysctl-amd64.conf");

/// Embedded architecture-specific early-boot sysctl policy.
#[cfg(target_arch = "aarch64")]
const SYSCTL_CONF: &str = include_str!("../../kernel/sysctl-arm64.conf");

/// Embedded architecture-specific early-boot sysctl policy.
#[cfg(not(any(target_arch = "x86_64", target_arch = "aarch64")))]
const SYSCTL_CONF: &str = "";

/// A sysctl key-value pair.
#[derive(Clone, Debug, Eq, PartialEq)]
struct Sysctl {
    key: String,
    value: String,
}

impl Sysctl {
    /// Creates a sysctl setting.
    fn new(key: String, value: String) -> Self {
        Self { key, value }
    }
}

/// Counts the results of applying sysctl settings.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct ApplySummary {
    pub applied: usize,
    pub unchanged: usize,
    pub skipped: usize,
}

/// The result of applying a single sysctl setting.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ApplyOutcome {
    Applied,
    Unchanged,
    Skipped,
}

/// Applies the configured sysctl settings.
pub fn apply() -> Result<ApplySummary> {
    let settings = parse(SYSCTL_CONF)?;
    apply_in(Path::new(PROC_SYS_DIR), &settings)
}

/// Applies sysctl settings in a procfs sysctl directory.
fn apply_in(proc_sys_dir: &Path, settings: &[Sysctl]) -> Result<ApplySummary> {
    if !proc_sys_dir.is_dir() {
        bail!(
            "Proc sysctl directory is unavailable at {}",
            proc_sys_dir.display()
        );
    }

    let mut summary = ApplySummary::default();
    for setting in settings {
        match apply_one(proc_sys_dir, setting)? {
            ApplyOutcome::Applied => summary.applied += 1,
            ApplyOutcome::Unchanged => summary.unchanged += 1,
            ApplyOutcome::Skipped => summary.skipped += 1,
        }
    }

    Ok(summary)
}

/// Applies a single sysctl setting.
fn apply_one(proc_sys_dir: &Path, setting: &Sysctl) -> Result<ApplyOutcome> {
    let path = sysctl_path(proc_sys_dir, &setting.key);
    let current = match fs::read_to_string(&path) {
        Ok(current) => current,
        Err(err) if err.kind() == ErrorKind::NotFound => return Ok(ApplyOutcome::Skipped),
        Err(err) => {
            return Err(err).with_context(|| format!("Failed to read sysctl {}", setting.key));
        }
    };

    if current.trim() == setting.value {
        return Ok(ApplyOutcome::Unchanged);
    }

    fs::write(&path, &setting.value)
        .with_context(|| format!("Failed to write sysctl {}={}", setting.key, setting.value))?;
    Ok(ApplyOutcome::Applied)
}

/// Parses sysctl settings from a config string.
fn parse(config: &str) -> Result<Vec<Sysctl>> {
    let mut settings = Vec::new();

    for (index, raw_line) in config.lines().enumerate() {
        let line = raw_line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }

        let Some((key, value)) = line.split_once('=') else {
            bail!("Invalid sysctl config line {}: {}", index + 1, raw_line);
        };

        let key = key.trim();
        let value = value.trim();
        if key.is_empty() || value.is_empty() {
            bail!("Invalid sysctl config line {}: {}", index + 1, raw_line);
        }

        settings.push(Sysctl::new(key.to_owned(), value.to_owned()));
    }

    Ok(settings)
}

/// Converts a dotted sysctl key to a procfs path.
fn sysctl_path(proc_sys_dir: &Path, key: &str) -> PathBuf {
    proc_sys_dir.join(key.replace('.', "/"))
}

#[cfg(test)]
mod tests {
    use tempfile::TempDir;

    use super::*;

    /// Writes a sysctl file in a temporary procfs tree.
    fn write_sysctl(proc_sys_dir: &Path, key: &str, value: &str) {
        let path = sysctl_path(proc_sys_dir, key);
        let parent = path
            .parent()
            .expect("Sysctl path should have a parent directory");
        fs::create_dir_all(parent).expect("Failed to create sysctl parent directories");
        fs::write(path, value).expect("Failed to write sysctl file");
    }

    /// Reads a sysctl file from a temporary procfs tree.
    fn read_sysctl(proc_sys_dir: &Path, key: &str) -> String {
        let path = sysctl_path(proc_sys_dir, key);
        fs::read_to_string(path).expect("Failed to read sysctl file")
    }

    #[test]
    fn sysctl_path_replaces_dots_with_separators() {
        // ARRANGE
        let proc_sys_dir = Path::new("/proc/sys");

        // ACT
        let path = sysctl_path(proc_sys_dir, "kernel.kptr_restrict");

        // ASSERT
        assert_eq!(path, Path::new("/proc/sys/kernel/kptr_restrict"));
    }

    #[test]
    fn parse_ignores_comments_and_blank_lines() {
        // ARRANGE
        let config = "\n# comment\nkernel.kptr_restrict=2\n\nfs.suid_dumpable=0\n";

        // ACT
        let settings = parse(config).expect("Failed to parse sysctl config");

        // ASSERT
        assert_eq!(settings.len(), 2);
        assert_eq!(settings[0].key, "kernel.kptr_restrict");
        assert_eq!(settings[0].value, "2");
        assert_eq!(settings[1].key, "fs.suid_dumpable");
        assert_eq!(settings[1].value, "0");
    }

    #[test]
    fn parse_rejects_invalid_lines() {
        // ARRANGE
        let config = "kernel.kptr_restrict 2\n";

        // ACT
        let error = parse(config).expect_err("Parsing should fail");

        // ASSERT
        assert_eq!(
            error.to_string(),
            "Invalid sysctl config line 1: kernel.kptr_restrict 2"
        );
    }

    #[test]
    fn apply_in_writes_changed_values() {
        // ARRANGE
        let temp = TempDir::new().expect("Failed to create temp dir");
        write_sysctl(temp.path(), "kernel.kptr_restrict", "1\n");
        let settings = [Sysctl::new(
            "kernel.kptr_restrict".to_owned(),
            "2".to_owned(),
        )];

        // ACT
        let summary = apply_in(temp.path(), &settings).expect("Failed to apply sysctls");

        // ASSERT
        assert_eq!(summary.applied, 1);
        assert_eq!(summary.unchanged, 0);
        assert_eq!(summary.skipped, 0);
        assert_eq!(read_sysctl(temp.path(), "kernel.kptr_restrict"), "2");
    }

    #[test]
    fn apply_in_leaves_matching_values_unchanged() {
        // ARRANGE
        let temp = TempDir::new().expect("Failed to create temp dir");
        write_sysctl(temp.path(), "fs.protected_symlinks", "1\n");
        let settings = [Sysctl::new(
            "fs.protected_symlinks".to_owned(),
            "1".to_owned(),
        )];

        // ACT
        let summary = apply_in(temp.path(), &settings).expect("Failed to apply sysctls");

        // ASSERT
        assert_eq!(summary.applied, 0);
        assert_eq!(summary.unchanged, 1);
        assert_eq!(summary.skipped, 0);
        assert_eq!(read_sysctl(temp.path(), "fs.protected_symlinks"), "1\n");
    }

    #[test]
    fn apply_in_skips_missing_nodes() {
        // ARRANGE
        let temp = TempDir::new().expect("Failed to create temp dir");
        let settings = [Sysctl::new(
            "kernel.missing_setting".to_owned(),
            "1".to_owned(),
        )];

        // ACT
        let summary = apply_in(temp.path(), &settings).expect("Failed to apply sysctls");

        // ASSERT
        assert_eq!(summary.applied, 0);
        assert_eq!(summary.unchanged, 0);
        assert_eq!(summary.skipped, 1);
    }

    #[test]
    fn apply_in_requires_proc_sys_directory() {
        // ARRANGE
        let missing = Path::new("/nonexistent/proc/sys");
        let settings = [Sysctl::new(
            "kernel.kptr_restrict".to_owned(),
            "2".to_owned(),
        )];

        // ACT
        let error = apply_in(missing, &settings).expect_err("Applying sysctls should fail");

        // ASSERT
        assert!(
            error
                .to_string()
                .contains("Proc sysctl directory is unavailable at /nonexistent/proc/sys")
        );
    }
}
