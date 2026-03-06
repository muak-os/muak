use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::process::Output;

use anyhow::{Context, Result, bail};
use tempfile::TempDir;

/// Drives the real `muakctl` binary with per-test isolation via a temporary config directory.
pub struct Cli {
    bin: PathBuf,
    config_dir: TempDir,
    endpoint: String,
}

impl Cli {
    /// Creates a new CLI driver pointing at the given host port.
    pub fn new(cli_bin: &Path, host_port: u16) -> Result<Self> {
        let config_dir = TempDir::new().context("failed to create temp config directory")?;
        Ok(Self {
            bin: cli_bin.to_path_buf(),
            config_dir,
            endpoint: format!("127.0.0.1:{host_port}"),
        })
    }

    /// Returns the path to the isolated MUAK_CONFIG file.
    pub fn config_path(&self) -> PathBuf {
        self.config_dir.path().join("config.toml")
    }

    /// Runs `muakctl` with the given arguments, injecting `--endpoint` and `MUAK_CONFIG`.
    pub fn run<I, S>(&self, args: I) -> Result<Output>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<OsStr>,
    {
        let output = std::process::Command::new(&self.bin)
            .env("MUAK_CONFIG", self.config_path())
            .env("HOME", self.config_dir.path())
            .arg("--endpoint")
            .arg(&self.endpoint)
            .args(args)
            .output()
            .with_context(|| format!("failed to execute {}", self.bin.display()))?;

        Ok(output)
    }

    /// Runs `muakctl` in insecure (TOFU) mode.
    pub fn run_insecure<I, S>(&self, args: I) -> Result<Output>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<OsStr>,
    {
        let output = std::process::Command::new(&self.bin)
            .env("MUAK_CONFIG", self.config_path())
            .env("HOME", self.config_dir.path())
            .arg("--endpoint")
            .arg(&self.endpoint)
            .arg("--insecure")
            .args(args)
            .output()
            .with_context(|| format!("failed to execute {}", self.bin.display()))?;

        Ok(output)
    }

    /// Runs a command and asserts it exits successfully, returning stdout as a string.
    pub fn assert_success<I, S>(&self, args: I) -> Result<String>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<OsStr>,
    {
        let output = self.run(args)?;
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            let stdout = String::from_utf8_lossy(&output.stdout);
            bail!(
                "muakctl exited with {}\n--- stdout ---\n{stdout}\n--- stderr ---\n{stderr}",
                output.status
            );
        }
        Ok(String::from_utf8_lossy(&output.stdout).into_owned())
    }

    /// Runs a command in insecure mode and asserts it exits successfully.
    pub fn assert_success_insecure<I, S>(&self, args: I) -> Result<String>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<OsStr>,
    {
        let output = self.run_insecure(args)?;
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            let stdout = String::from_utf8_lossy(&output.stdout);
            bail!(
                "muakctl --insecure exited with {}\n--- stdout ---\n{stdout}\n--- stderr ---\n{stderr}",
                output.status
            );
        }
        Ok(String::from_utf8_lossy(&output.stdout).into_owned())
    }

    /// Runs a command and asserts the stdout contains the given needle.
    pub fn assert_output_contains<I, S>(&self, args: I, needle: &str) -> Result<()>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<OsStr>,
    {
        let stdout = self.assert_success(args)?;
        if !stdout.contains(needle) {
            bail!("stdout does not contain '{needle}'\n--- stdout ---\n{stdout}");
        }
        Ok(())
    }

    /// Runs a command in insecure mode and asserts the stdout contains the given needle.
    pub fn assert_output_contains_insecure<I, S>(&self, args: I, needle: &str) -> Result<()>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<OsStr>,
    {
        let stdout = self.assert_success_insecure(args)?;
        if !stdout.contains(needle) {
            bail!("stdout does not contain '{needle}'\n--- stdout ---\n{stdout}");
        }
        Ok(())
    }
}
