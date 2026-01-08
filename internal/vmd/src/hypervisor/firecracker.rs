use anyhow::Result;
use tokio::process::Command;

use super::{VmProcess, VmStartConfig};

pub struct FirecrackerHypervisor {
    binary_path: String,
}

impl FirecrackerHypervisor {
    pub fn new() -> Self {
        Self {
            binary_path: "/usr/bin/firecracker".to_string(),
        }
    }

    pub async fn start(&self, config: &VmStartConfig) -> Result<VmProcess> {
        let config_path = format!("/run/vmd/{}/config.json", config.vm_id);

        let mut drives: Vec<_> = config
            .disks
            .iter()
            .enumerate()
            .map(|(i, d)| {
                serde_json::json!({
                    "drive_id": format!("disk{}", i),
                    "path_on_host": d.path.to_string_lossy(),
                    "is_root_device": i == 0,
                    "is_read_only": d.readonly,
                })
            })
            .collect();

        if let Some(persistent_disk) = &config.persistent_disk {
            drives.push(serde_json::json!({
                "drive_id": "persistent",
                "path_on_host": persistent_disk.to_string_lossy(),
                "is_root_device": false,
                "is_read_only": false,
            }));
        }

        let fc_config = serde_json::json!({
            "boot-source": {
                "kernel_image_path": config.kernel.to_string_lossy(),
                "boot_args": config.cmdline,
                "initrd_path": config.initrd.as_ref().map(|p| p.to_string_lossy().to_string()),
            },
            "machine-config": {
                "vcpu_count": config.cpus,
                "mem_size_mib": config.memory_mb,
            },
            "network-interfaces": [{
                "iface_id": "eth0",
                "guest_mac": config.mac_address,
                "host_dev_name": config.tap_device,
            }],
            "drives": drives,
        });

        let config_dir = std::path::Path::new(&config_path)
            .parent()
            .ok_or_else(|| anyhow::anyhow!("Invalid config path: {}", config_path))?;
        tokio::fs::create_dir_all(config_dir).await?;
        tokio::fs::write(&config_path, serde_json::to_string_pretty(&fc_config)?).await?;

        let child = Command::new(&self.binary_path)
            .arg("--config-file")
            .arg(&config_path)
            .arg("--log-path")
            .arg(&config.serial_log_path)
            .spawn()?;

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
