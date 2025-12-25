use anyhow::Result;
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

        let child = cmd.spawn()?;
        let pid = child
            .id()
            .ok_or_else(|| anyhow::anyhow!("Failed to get child PID"))?;

        Ok(VmProcess { pid })
    }

    pub async fn stop(&self, pid: u32, force: bool) -> Result<()> {
        use nix::sys::signal::{Signal, kill};
        use nix::unistd::Pid;

        let signal = if force {
            Signal::SIGKILL
        } else {
            Signal::SIGTERM
        };
        kill(Pid::from_raw(pid as i32), signal)?;
        Ok(())
    }
}
