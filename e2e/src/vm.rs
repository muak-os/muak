use std::path::PathBuf;
use std::time::Duration;

use anyhow::{Context, Result, bail};
use tempfile::NamedTempFile;
use tokio::process::{Child, Command};
use tokio::time::{sleep, timeout};

use crate::artifacts::Artifacts;
use crate::port;

const APID_GUEST_PORT: u16 = 50051;
const APID_READY_MARKER: &str = "[apid] API daemon ready, listening on";

/// Boot mode for the QEMU VM.
pub enum BootMode {
    Live,
    Install { disk_gib: u64 },
}

/// A running QEMU VM with port-forwarded access to apid.
pub struct QemuVm {
    process: Child,
    pub host_port: u16,
    pub serial_log: PathBuf,
    pub qemu_stderr_log: PathBuf,
    _serial_file: NamedTempFile,
    _qemu_stderr_file: NamedTempFile,
    _ovmf_vars: NamedTempFile,
    _disk: Option<NamedTempFile>,
}

impl QemuVm {
    /// Boots a VM in live mode (ISO only, no persistent disk).
    pub async fn boot_live(artifacts: &Artifacts) -> Result<Self> {
        Self::boot(artifacts, BootMode::Live).await
    }

    /// Boots a VM in install mode (ISO + NVMe disk).
    pub async fn boot_install(artifacts: &Artifacts, disk_gib: u64) -> Result<Self> {
        Self::boot(artifacts, BootMode::Install { disk_gib }).await
    }

    async fn boot(artifacts: &Artifacts, mode: BootMode) -> Result<Self> {
        let host_port = port::allocate()?;

        let serial_file = NamedTempFile::new().context("failed to create serial log tempfile")?;
        let serial_path = serial_file.path().to_path_buf();

        let stderr_file = NamedTempFile::new().context("failed to create QEMU stderr tempfile")?;
        let stderr_path = stderr_file.path().to_path_buf();

        let ovmf_vars = NamedTempFile::new().context("failed to create OVMF VARS tempfile")?;
        std::fs::copy(&artifacts.ovmf_vars, ovmf_vars.path())
            .context("failed to copy OVMF_VARS.secboot.fd")?;

        let hostfwd = format!("user,id=net0,hostfwd=tcp:127.0.0.1:{host_port}-:{APID_GUEST_PORT}");

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
            .arg("-cdrom")
            .arg(&artifacts.iso)
            .arg("-netdev")
            .arg(&hostfwd)
            .arg("-device")
            .arg("virtio-net-pci,netdev=net0")
            .arg("-serial")
            .arg(format!("file:{}", serial_path.display()))
            .arg("-display")
            .arg("none")
            .arg("-no-reboot");

        let disk = match &mode {
            BootMode::Live => None,
            BootMode::Install { disk_gib } => {
                let disk_file =
                    NamedTempFile::new().context("failed to create NVMe disk tempfile")?;
                disk_file
                    .as_file()
                    .set_len(disk_gib * 1024 * 1024 * 1024)
                    .context("failed to allocate NVMe disk image")?;

                cmd.arg("-drive")
                    .arg(format!(
                        "file={},format=raw,if=none,id=nvme0",
                        disk_file.path().display()
                    ))
                    .arg("-device")
                    .arg("nvme,serial=deadbeef,drive=nvme0");

                Some(disk_file)
            }
        };

        cmd.stdout(std::process::Stdio::null()).stderr(
            stderr_file
                .as_file()
                .try_clone()
                .context("failed to clone QEMU stderr file handle")?,
        );

        let process = cmd.spawn().context("failed to spawn qemu-system-x86_64")?;

        Ok(Self {
            process,
            host_port,
            serial_log: serial_path,
            qemu_stderr_log: stderr_path,
            _serial_file: serial_file,
            _qemu_stderr_file: stderr_file,
            _ovmf_vars: ovmf_vars,
            _disk: disk,
        })
    }

    /// Waits until apid is ready by polling the serial log for the ready marker.
    pub async fn wait_ready(&self, dur: Duration) -> Result<()> {
        timeout(dur, self.poll_serial_ready()).await.map_err(|_| {
            let serial = self.read_serial().unwrap_or_default();
            let qemu_err = self.read_qemu_stderr().unwrap_or_default();
            anyhow::anyhow!(
                "VM did not become ready within {}s (no '{}' in serial log)\
                     \n\n--- serial log ---\n{serial}\
                     \n\n--- qemu stderr ---\n{qemu_err}",
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

    /// Waits until the forwarded TCP port stops accepting connections (e.g. during reboot).
    pub async fn wait_port_closed(&self, dur: Duration) -> Result<()> {
        let addr = format!("127.0.0.1:{}", self.host_port);
        timeout(dur, poll_tcp_closed(&addr)).await.map_err(|_| {
            anyhow::anyhow!(
                "port {} did not close within {}s",
                self.host_port,
                dur.as_secs()
            )
        })?
    }

    /// Reads the full serial console log captured from the VM.
    pub fn read_serial(&self) -> Result<String> {
        std::fs::read_to_string(&self.serial_log).context("failed to read serial log")
    }

    /// Returns the last `n` lines of the serial log.
    pub fn tail_serial(&self, n: usize) -> Result<String> {
        let log = self.read_serial()?;
        let lines: Vec<&str> = log.lines().collect();
        let start = lines.len().saturating_sub(n);
        Ok(lines[start..].join("\n"))
    }

    /// Reads the QEMU process stderr log.
    pub fn read_qemu_stderr(&self) -> Result<String> {
        std::fs::read_to_string(&self.qemu_stderr_log).context("failed to read QEMU stderr log")
    }

    /// Asserts that the serial log contains the given substring.
    pub fn assert_serial_contains(&self, needle: &str) -> Result<()> {
        let log = self.read_serial()?;
        if !log.contains(needle) {
            bail!("serial log does not contain '{needle}'\n\n--- serial log ---\n{log}");
        }
        Ok(())
    }

    /// Sends SIGKILL to the QEMU process.
    pub async fn kill(&mut self) -> Result<()> {
        self.process
            .kill()
            .await
            .context("failed to kill QEMU process")
    }
}

impl Drop for QemuVm {
    fn drop(&mut self) {
        let _ = self.process.start_kill();
    }
}

async fn poll_tcp_closed(addr: &str) -> Result<()> {
    loop {
        if tokio::net::TcpStream::connect(addr).await.is_err() {
            return Ok(());
        }
        sleep(Duration::from_millis(500)).await;
    }
}
