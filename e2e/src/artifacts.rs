use std::path::PathBuf;

use anyhow::{Result, bail, ensure};

/// Resolved paths to all artifacts.
pub struct Artifacts {
    pub iso: PathBuf,
    pub ovmf_code: PathBuf,
    pub ovmf_vars: PathBuf,
    pub cli_bin: PathBuf,
}

impl Artifacts {
    /// Resolves artifacts from environment variables.
    ///
    /// # Errors
    ///
    /// Returns an error if a required artifact file is missing or `qemu-system-x86_64` is not
    /// available in `PATH`.
    pub fn from_env() -> Result<Self> {
        let workspace = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..");

        let artifacts_dir =
            std::env::var("MUAK_ARTIFACTS").map_or_else(|_| workspace.join("_out"), PathBuf::from);

        let iso = artifacts_dir.join("muak.iso");
        ensure!(
            iso.exists(),
            "muak.iso not found at {}\nRun `just dev` to build artifacts, \
             or set MUAK_ARTIFACTS to the directory containing them.",
            iso.display()
        );

        let ovmf_code = artifacts_dir.join("OVMF_CODE.secboot.fd");
        ensure!(
            ovmf_code.exists(),
            "OVMF_CODE.secboot.fd not found at {}\nRun `just e2e` to fetch it automatically.",
            ovmf_code.display()
        );

        let ovmf_vars = artifacts_dir.join("OVMF_VARS.fd");
        ensure!(
            ovmf_vars.exists(),
            "OVMF_VARS.fd not found at {}\nRun `just e2e` to fetch it automatically.",
            ovmf_vars.display()
        );

        let cli_bin = std::env::var("MUAK_CLI").map_or_else(
            |_| {
                let arch = std::env::consts::ARCH;
                workspace.join(format!("target/{arch}-unknown-linux-musl/release/muakctl"))
            },
            PathBuf::from,
        );
        ensure!(
            cli_bin.exists(),
            "muakctl binary not found at {}.\nRun `just build --release muakctl` or set MUAK_CLI to the binary path.",
            cli_bin.display()
        );

        ensure_qemu_available()?;

        Ok(Self {
            iso,
            ovmf_code,
            ovmf_vars,
            cli_bin,
        })
    }
}

fn ensure_qemu_available() -> Result<()> {
    let status = std::process::Command::new("qemu-system-x86_64")
        .arg("--version")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status();

    match status {
        Ok(status) if status.success() => Ok(()),
        _ => bail!("qemu-system-x86_64 not found in PATH."),
    }
}
