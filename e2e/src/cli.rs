use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::process::Output;

use anyhow::{Context as _, Result, bail};
use config::SystemConfig;
use tempfile::{NamedTempFile, TempDir};
use tokio::process::Command;

/// Drives the real `muakctl` binary with per-test isolation via a temporary config directory.
pub struct Cli {
    bin: PathBuf,
    config_dir: TempDir,
    endpoint: String,
}

impl Cli {
    /// Creates a new CLI driver pointing at the given host port.
    ///
    /// # Errors
    ///
    /// Returns an error if the temporary config directory cannot be created.
    pub fn new(cli_bin: &Path, host_port: u16) -> Result<Self> {
        let config_dir = TempDir::new().context("failed to create temp config directory")?;
        Ok(Self {
            bin: cli_bin.to_path_buf(),
            config_dir,
            endpoint: format!("127.0.0.1:{host_port}"),
        })
    }

    /// Returns the path to the isolated `MUAK_CONFIG` file.
    #[must_use]
    pub fn config_path(&self) -> PathBuf {
        self.config_dir
            .path()
            .join(format!("config.{}", config::CONFIG_EXTENSION))
    }

    /// Runs `muakctl` with the given arguments, injecting `--endpoint` and `MUAK_CONFIG`.
    /// Pass `insecure: true` to also add `--insecure` (TOFU mode).
    ///
    /// # Errors
    ///
    /// Returns an error if `muakctl` fails to execute or the command fails.
    pub async fn run<I, S>(&self, args: I, insecure: bool) -> Result<Output>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<OsStr>,
    {
        let mut cmd = Command::new(&self.bin);
        cmd.env("MUAK_CONFIG", self.config_path())
            .env("HOME", self.config_dir.path())
            .env("NO_COLOR", "1")
            .arg("--endpoint")
            .arg(&self.endpoint);
        if insecure {
            cmd.arg("--insecure");
        }
        cmd.args(args)
            .output()
            .await
            .with_context(|| format!("failed to execute {}", self.bin.display()))
    }

    #[doc(hidden)]
    pub async fn assert_success_impl<I, S>(&self, args: I, insecure: bool) -> Result<String>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<OsStr>,
    {
        let flag = if insecure { " --insecure" } else { "" };
        let output = self.run(args, insecure).await?;
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            let stdout = String::from_utf8_lossy(&output.stdout);
            bail!(
                "muakctl{flag} exited with {}\n--- stdout ---\n{stdout}\n--- stderr ---\n{stderr}",
                output.status
            );
        }
        Ok(String::from_utf8_lossy(&output.stdout).into_owned())
    }

    #[doc(hidden)]
    pub async fn assert_output_contains_impl<I, S>(
        &self,
        args: I,
        needle: &str,
        insecure: bool,
    ) -> Result<()>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<OsStr>,
    {
        let stdout = self.assert_success_impl(args, insecure).await?;
        if !stdout.contains(needle) {
            bail!("stdout does not contain '{needle}'\n--- stdout ---\n{stdout}");
        }
        Ok(())
    }

    /// Runs `muakctl config generate`, applies `patch`, and writes the result to a temp file.
    ///
    /// # Errors
    ///
    /// Returns an error if `muakctl config generate` fails, the config cannot be parsed or
    /// serialised, or the temporary file cannot be written.
    pub async fn generate_config<F: FnOnce(&mut SystemConfig)>(
        &self,
        patch: F,
    ) -> Result<NamedTempFile> {
        let raw = self
            .assert_success_impl(["config", "generate"], false)
            .await?;
        let mut cfg: SystemConfig =
            config::parse_from_str(&raw).context("failed to parse generated config")?;
        patch(&mut cfg);
        let patched = config::serialize(&cfg).context("failed to serialise patched config")?;
        let tmp = NamedTempFile::new().context("failed to create config tempfile")?;
        std::fs::write(tmp.path(), patched).context("failed to write config tempfile")?;
        Ok(tmp)
    }
}

/// Asserts that `muakctl` exits successfully and returns stdout.
///
/// Usage: `assert_success!(cli, ["arg1", "arg2"])`.
#[macro_export]
macro_rules! assert_success {
    ($cli:expr, $args:expr) => {
        $cli.assert_success_impl($args, false)
    };
}

/// Asserts that `muakctl --insecure` exits successfully and returns stdout.
///
/// Usage: `assert_success_insecure!(cli, ["arg1", "arg2"])`.
#[macro_export]
macro_rules! assert_success_insecure {
    ($cli:expr, $args:expr) => {
        $cli.assert_success_impl($args, true)
    };
}

/// Asserts that `muakctl` stdout contains the given needle.
#[macro_export]
macro_rules! assert_output_contains {
    ($cli:expr, $args:expr, $needle:expr) => {
        $cli.assert_output_contains_impl($args, $needle, false)
    };
}

/// Asserts that `muakctl --insecure` stdout contains the given needle.
///
/// Usage: `assert_output_contains_insecure!(cli, ["arg1"], "needle")`.
#[macro_export]
macro_rules! assert_output_contains_insecure {
    ($cli:expr, $args:expr, $needle:expr) => {
        $cli.assert_output_contains_impl($args, $needle, true)
    };
}
