use core::time::Duration;
use std::path::PathBuf;

use anyhow::{Context as _, Result, bail};
use tempfile::NamedTempFile;
use tokio::process::{Child, Command};
use tokio::time::{sleep, timeout};

use crate::artifacts::Artifacts;
use crate::port;

/// Wraps a [`Vm`] and dumps the serial log and stderr when panic occurs.
pub struct TestFixture {
    pub vm: Vm,
}

impl TestFixture {
    /// Boots a VM in live mode and wraps it in a fixture that dumps logs on panic.
    ///
    /// # Errors
    ///
    /// Returns an error if the VM cannot be booted.
    pub fn boot_live(artifacts: &Artifacts) -> Result<Self> {
        Ok(Self {
            vm: Vm::boot_live(artifacts)?,
        })
    }

    /// Boots a VM in install mode and wraps it in a fixture that dumps logs on panic.
    ///
    /// # Errors
    ///
    /// Returns an error if the VM cannot be booted.
    pub fn boot_install(artifacts: &Artifacts) -> Result<Self> {
        Ok(Self {
            vm: Vm::boot_install(artifacts)?,
        })
    }
}

impl Drop for TestFixture {
    fn drop(&mut self) {
        if std::thread::panicking() {
            let serial = self
                .vm
                .read_serial()
                .unwrap_or_else(|e| format!("<error: {e}>"));
            let stderr = self
                .vm
                .read_stderr()
                .unwrap_or_else(|e| format!("<error: {e}>"));
            eprintln!("\n--- serial log ---\n{serial}\n--- stderr ---\n{stderr}");
        }
    }
}

pub const APID_PORT: u16 = 50051;
const APID_READY_MARKER: &str = "[apid] API daemon ready, listening on";

/// Boot mode for the VM.
#[derive(Clone, Copy)]
pub enum BootMode {
    Live,
    Install,
}

/// A running VM with port-forwarded access to apid.
pub struct Vm {
    pub process: Child,
    pub host_port: u16,
    pub serial_log: PathBuf,
    pub stderr_log: PathBuf,
    pub serial_file: NamedTempFile,
    pub stderr_file: NamedTempFile,
    pub ovmf_vars: NamedTempFile,
    pub disk: Option<NamedTempFile>,
}

impl Vm {
    /// Boots a VM in live mode (ISO only, no persistent disk).
    ///
    /// # Errors
    ///
    /// Returns an error if port allocation fails, temp files cannot be created, OVMF vars cannot
    /// be copied, or QEMU fails to spawn.
    pub fn boot_live(artifacts: &Artifacts) -> Result<Self> {
        Self::boot(artifacts, BootMode::Live)
    }

    /// Boots a VM in install mode (ISO + `NVMe` disk).
    ///
    /// # Errors
    ///
    /// Returns an error if port allocation fails, temp files cannot be created, OVMF vars cannot
    /// be copied, or QEMU fails to spawn.
    pub fn boot_install(artifacts: &Artifacts) -> Result<Self> {
        Self::boot(artifacts, BootMode::Install)
    }

    fn boot(artifacts: &Artifacts, mode: BootMode) -> Result<Self> {
        let host_port = port::allocate()?;

        let serial_file = NamedTempFile::new().context("failed to create serial log tempfile")?;
        let serial_path = serial_file.path().to_path_buf();

        let stderr_file = NamedTempFile::new().context("failed to create stderr tempfile")?;
        let stderr_path = stderr_file.path().to_path_buf();

        let ovmf_vars = NamedTempFile::new().context("failed to create OVMF VARS tempfile")?;
        std::fs::copy(&artifacts.ovmf_vars, ovmf_vars.path())
            .context("failed to copy OVMF_VARS.secboot.fd")?;

        let hostfwd = format!("user,id=net0,hostfwd=tcp:127.0.0.1:{host_port}-:{APID_PORT}");

        let mut cmd = Command::new("qemu-system-x86_64");
        cmd.arg("-enable-kvm")
            .arg("-machine")
            .arg("type=q35,accel=kvm")
            .arg("-cpu")
            .arg("host")
            .arg("-m")
            .arg("2G")
            .arg("-smp")
            .arg("2")
            .arg("-drive")
            .arg(format!(
                "if=pflash,format=raw,readonly=on,file={}",
                artifacts.ovmf_code.display()
            ))
            .arg("-drive")
            .arg(format!(
                "if=pflash,format=raw,file={}",
                ovmf_vars.path().display()
            ))
            .arg("-netdev")
            .arg(&hostfwd)
            .arg("-device")
            .arg("virtio-net-pci,netdev=net0")
            .arg("-serial")
            .arg(format!("file:{}", serial_path.display()))
            .arg("-display")
            .arg("none");

        match mode {
            BootMode::Live => {
                cmd.arg("-drive")
                    .arg(format!(
                        "file={},format=raw,media=cdrom,if=none,id=cdrom0,readonly=on",
                        artifacts.iso.display()
                    ))
                    .arg("-device")
                    .arg("ide-cd,drive=cdrom0,bootindex=1")
                    .arg("-no-reboot");
            }
            BootMode::Install => {
                cmd.arg("-drive")
                    .arg(format!(
                        "file={},format=raw,media=cdrom,if=none,id=cdrom0,readonly=on",
                        artifacts.iso.display()
                    ))
                    .arg("-device")
                    .arg("ide-cd,drive=cdrom0,bootindex=2");
            }
        }

        let disk = match mode {
            BootMode::Live => None,
            BootMode::Install => {
                let disk_file =
                    NamedTempFile::new().context("failed to create NVMe disk tempfile")?;
                disk_file
                    .as_file()
                    .set_len(5 * 1024 * 1024 * 1024)
                    .context("failed to allocate NVMe disk image")?;

                cmd.arg("-drive")
                    .arg(format!(
                        "file={},format=raw,if=none,id=nvme0",
                        disk_file.path().display()
                    ))
                    .arg("-device")
                    .arg("nvme,serial=deadbeef,drive=nvme0,bootindex=1");

                Some(disk_file)
            }
        };

        cmd.stdout(std::process::Stdio::null()).stderr(
            stderr_file
                .as_file()
                .try_clone()
                .context("failed to clone stderr file handle")?,
        );

        let process = cmd.spawn().context("failed to spawn qemu-system-x86_64")?;

        Ok(Self {
            process,
            host_port,
            serial_log: serial_path,
            stderr_log: stderr_path,
            serial_file,
            stderr_file,
            ovmf_vars,
            disk,
        })
    }

    /// Waits until apid is ready by polling the serial log for the ready marker.
    ///
    /// # Errors
    ///
    /// Returns an error if the timeout elapses before the ready marker appears in the serial log.
    pub async fn wait_ready(&self, dur: Duration) -> Result<()> {
        timeout(dur, self.poll_serial_ready())
            .await
            .map_err(|_elapsed| {
                let serial = self.read_serial().unwrap_or_default();
                let stderr = self.read_stderr().unwrap_or_default();
                anyhow::anyhow!(
                    "VM did not become ready within {}s (no '{}' in serial log)\
                     \n\n--- serial log ---\n{serial}\
                     \n\n--- stderr ---\n{stderr}",
                    dur.as_secs(),
                    APID_READY_MARKER,
                )
            })
    }

    async fn poll_serial_ready(&self) {
        while !self
            .read_serial()
            .is_ok_and(|log| log.contains(APID_READY_MARKER))
        {
            sleep(Duration::from_millis(200)).await;
        }
    }

    /// Reads the full serial console log captured from the VM.
    ///
    /// # Errors
    ///
    /// Returns an error if the serial log file cannot be read.
    pub fn read_serial(&self) -> Result<String> {
        let raw = std::fs::read(&self.serial_log).context("failed to read serial log")?;
        Ok(String::from_utf8_lossy(raw.trim_ascii()).replace('\0', ""))
    }

    /// Reads the process stderr log.
    ///
    /// # Errors
    ///
    /// Returns an error if the stderr log file cannot be read.
    pub fn read_stderr(&self) -> Result<String> {
        let raw = std::fs::read(&self.stderr_log).context("failed to read stderr log")?;
        Ok(String::from_utf8_lossy(raw.trim_ascii()).replace('\0', ""))
    }

    /// Waits until the serial log contains `needle`, or the timeout elapses.
    ///
    /// # Errors
    ///
    /// Returns an error if the timeout elapses before `needle` appears in the serial log.
    pub async fn wait_serial_contains(&self, needle: &str, dur: Duration) -> Result<()> {
        timeout(dur, self.poll_serial_contains(needle))
            .await
            .map_err(|_elapsed| {
                let serial = self.read_serial().unwrap_or_default();
                anyhow::anyhow!(
                    "serial log did not contain '{needle}' within {}s\
                     \n\n--- serial log ---\n{serial}",
                    dur.as_secs(),
                )
            })
    }

    async fn poll_serial_contains(&self, needle: &str) {
        while !self.read_serial().is_ok_and(|log| log.contains(needle)) {
            sleep(Duration::from_millis(200)).await;
        }
    }

    /// Asserts that the serial log contains the given substring.
    ///
    /// # Errors
    ///
    /// Returns an error if the serial log cannot be read or does not contain `needle`.
    pub fn assert_serial_contains(&self, needle: &str) -> Result<()> {
        let log = self.read_serial()?;
        if !log.contains(needle) {
            bail!("serial log does not contain '{needle}'\n\n--- serial log ---\n{log}");
        }
        Ok(())
    }
}

impl Drop for Vm {
    fn drop(&mut self) {
        let _result = self.process.start_kill();
    }
}
