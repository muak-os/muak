use anyhow::Result;
use rustix::process::{Pid, Signal, kill_process};
use tokio::process::Command;

use super::{VmProcess, VmStartConfig};

fn readonly_flag(readonly: bool) -> &'static str {
    if readonly { ",readonly=on" } else { "" }
}

/// Returns the QEMU system binary path for the current host architecture.
fn qemu_binary_path() -> &'static str {
    match std::env::consts::ARCH {
        "aarch64" => "/usr/bin/qemu-system-aarch64",
        _ => "/usr/bin/qemu-system-x86_64",
    }
}

pub struct QemuHypervisor {
    binary_path: String,
}

impl QemuHypervisor {
    pub fn new() -> Self {
        Self {
            binary_path: qemu_binary_path().to_string(),
        }
    }

    pub async fn start(&self, config: &VmStartConfig) -> Result<VmProcess> {
        let mut cmd = Command::new(&self.binary_path);

        cmd.arg("-enable-kvm");
        cmd.arg("-cpu").arg("host");
        cmd.arg("-m").arg(format!("{}M", config.memory_mb));
        cmd.arg("-smp").arg(config.cpus.to_string());
        cmd.arg("-kernel").arg(&config.kernel);
        cmd.arg("-append").arg(&config.cmdline);
        cmd.arg("-nographic");
        cmd.arg("-serial")
            .arg(format!("file:{}", config.serial_log_path.display()));

        // ARM64-specific: use virtio-mmio machine type
        #[cfg(target_arch = "aarch64")]
        {
            cmd.arg("-machine").arg("virt,gic-version=3");
        }

        if let Some(initrd) = &config.initrd {
            cmd.arg("-initrd").arg(initrd);
        }

        cmd.arg("-netdev").arg(format!(
            "tap,id=net0,ifname={},script=no,downscript=no",
            config.tap_device
        ));
        cmd.arg("-device").arg(format!(
            "virtio-net-pci,netdev=net0,mac={}",
            config.mac_address
        ));

        for (i, disk) in config.disks.iter().enumerate() {
            cmd.arg("-drive").arg(format!(
                "file={},format=raw,if=none,id=disk{}{}",
                disk.path.display(),
                i,
                readonly_flag(disk.readonly)
            ));
            cmd.arg("-device")
                .arg(format!("virtio-blk-pci,drive=disk{}", i));
        }

        if let Some(persistent_disk) = &config.persistent_disk {
            let disk_idx = config.disks.len();
            cmd.arg("-drive").arg(format!(
                "file={},format=raw,if=none,id=disk{}",
                persistent_disk.display(),
                disk_idx
            ));
            cmd.arg("-device")
                .arg(format!("virtio-blk-pci,drive=disk{}", disk_idx));
        }

        cmd.arg("-no-reboot");

        let child = cmd.spawn()?;
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
