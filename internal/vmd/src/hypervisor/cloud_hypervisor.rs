use anyhow::Result;
use rustix::process::{Pid, Signal, kill_process};
use tokio::process::Command;

use super::{VmProcess, VmStartConfig};

pub struct CloudHypervisorHypervisor {
    binary_path: String,
}

impl CloudHypervisorHypervisor {
    pub fn new() -> Self {
        Self {
            binary_path: "/usr/bin/cloud-hypervisor".to_string(),
        }
    }

    pub async fn start(&self, config: &VmStartConfig) -> Result<VmProcess> {
        // Check all required files exist before spawning
        if !std::path::Path::new(&self.binary_path).exists() {
            anyhow::bail!("cloud-hypervisor binary not found at {}", self.binary_path);
        }

        if !config.kernel.exists() {
            anyhow::bail!("Kernel not found at {}", config.kernel.display());
        }

        if let Some(initrd) = &config.initrd
            && !initrd.exists()
        {
            anyhow::bail!("Initrd not found at {}", initrd.display());
        }

        if let Some(parent) = config.serial_log_path.parent()
            && !parent.exists()
        {
            anyhow::bail!("Serial log directory not found: {}", parent.display());
        }

        for disk in &config.disks {
            if !disk.path.exists() {
                anyhow::bail!("Disk not found at {}", disk.path.display());
            }
        }

        if let Some(persistent_disk) = &config.persistent_disk
            && !persistent_disk.exists()
        {
            anyhow::bail!("Persistent disk not found at {}", persistent_disk.display());
        }

        let mut cmd = Command::new(&self.binary_path);

        cmd.arg("--kernel").arg(&config.kernel);
        cmd.arg("--cmdline").arg(&config.cmdline);
        cmd.arg("--cpus").arg(format!("boot={}", config.cpus));
        cmd.arg("--memory")
            .arg(format!("size={}M", config.memory_mb));
        cmd.arg("--serial")
            .arg(format!("file={}", config.serial_log_path.display()));
        cmd.arg("--console").arg("off");

        if let Some(initrd) = &config.initrd {
            cmd.arg("--initramfs").arg(initrd);
        }

        cmd.arg("--net").arg(format!(
            "tap={},mac={}",
            config.tap_device, config.mac_address
        ));

        for disk in &config.disks {
            let readonly_flag = if disk.readonly { ",readonly=on" } else { "" };
            let disk_arg = format!("path={}{}", disk.path.display(), readonly_flag);
            cmd.arg("--disk").arg(disk_arg);
        }

        if let Some(persistent_disk) = &config.persistent_disk {
            cmd.arg("--disk")
                .arg(format!("path={}", persistent_disk.display()));
        }

        let child = cmd.spawn().map_err(|e| {
            anyhow::anyhow!(
                "Failed to spawn cloud-hypervisor: {} (binary: {}, kernel: {})",
                e,
                self.binary_path,
                config.kernel.display()
            )
        })?;
        let pid = child
            .id()
            .ok_or_else(|| anyhow::anyhow!("Failed to get child PID"))?;

        Ok(VmProcess { pid })
    }

    pub async fn stop(&self, pid: u32, force: bool) -> Result<()> {
        let signal = if force { Signal::KILL } else { Signal::TERM };
        kill_process(
            Pid::from_raw(pid as i32).ok_or_else(|| anyhow::anyhow!("Invalid PID"))?,
            signal,
        )?;
        Ok(())
    }
}
