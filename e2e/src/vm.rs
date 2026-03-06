use std::path::PathBuf;
use std::time::Duration;

use anyhow::{Context, Result, bail};
use tempfile::NamedTempFile;
use tokio::process::{Child, Command};
use tokio::time::{sleep, timeout};

use crate::artifacts::Artifacts;
use crate::port;

/// RAII guard that wraps a [`TestVm`] and dumps the serial log and stderr to
/// the test's stderr whenever the test thread is panicking (i.e. on failure).
pub struct TestFixture {
    pub vm: TestVm,
}

impl TestFixture {
    pub async fn boot_live(artifacts: &Artifacts) -> Result<Self> {
        Ok(Self {
            vm: TestVm::boot_live(artifacts).await?,
        })
    }

    pub async fn boot_install(artifacts: &Artifacts, disk_gib: u64) -> Result<Self> {
        Ok(Self {
            vm: TestVm::boot_install(artifacts, disk_gib).await?,
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

pub const APID_GUEST_PORT: u16 = 50051;
const APID_READY_MARKER: &str = "[apid] API daemon ready, listening on";

/// Boot mode for the VM.
pub enum BootMode {
    Live,
    Install { disk_gib: u64 },
}

/// A running VM with port-forwarded access to apid.
pub struct TestVm {
    process: Child,
    pub host_port: u16,
    pub serial_log: PathBuf,
    pub stderr_log: PathBuf,
    _serial_file: NamedTempFile,
    _stderr_file: NamedTempFile,
    _ovmf_vars: NamedTempFile,
    _disk: Option<NamedTempFile>,
}

impl TestVm {
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

        let stderr_file = NamedTempFile::new().context("failed to create stderr tempfile")?;
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
            BootMode::Install { .. } => {
                cmd.arg("-drive")
                    .arg(format!(
                        "file={},format=raw,media=cdrom,if=none,id=cdrom0,readonly=on",
                        artifacts.iso.display()
                    ))
                    .arg("-device")
                    .arg("ide-cd,drive=cdrom0,bootindex=2");
            }
        }

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
            _serial_file: serial_file,
            _stderr_file: stderr_file,
            _ovmf_vars: ovmf_vars,
            _disk: disk,
        })
    }

    /// Waits until apid is ready by polling the serial log for the ready marker.
    pub async fn wait_ready(&self, dur: Duration) -> Result<()> {
        timeout(dur, self.poll_serial_ready()).await.map_err(|_| {
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
        let raw = std::fs::read(&self.serial_log).context("failed to read serial log")?;
        Ok(String::from_utf8_lossy(raw.trim_ascii()).replace('\0', ""))
    }

    /// Returns the last `n` lines of the serial log.
    pub fn tail_serial(&self, n: usize) -> Result<String> {
        let log = self.read_serial()?;
        let lines: Vec<&str> = log.lines().collect();
        let start = lines.len().saturating_sub(n);
        Ok(lines[start..].join("\n"))
    }

    /// Reads the process stderr log.
    pub fn read_stderr(&self) -> Result<String> {
        let raw = std::fs::read(&self.stderr_log).context("failed to read stderr log")?;
        Ok(String::from_utf8_lossy(raw.trim_ascii()).replace('\0', ""))
    }

    /// Asserts that the serial log contains the given substring.
    pub fn assert_serial_contains(&self, needle: &str) -> Result<()> {
        let log = self.read_serial()?;
        if !log.contains(needle) {
            bail!("serial log does not contain '{needle}'\n\n--- serial log ---\n{log}");
        }
        Ok(())
    }

    /// Sends SIGKILL to the process.
    pub async fn kill(&mut self) -> Result<()> {
        self.process.kill().await.context("failed to kill process")
    }
}

impl Drop for TestVm {
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
