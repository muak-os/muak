use super::{VmmConfig, VmmStartResult};
use crate::process::ProcessManager;
use std::collections::HashMap;

pub struct CloudHypervisorBackend {
    binary_path: String,
}

impl CloudHypervisorBackend {
    pub fn new() -> Self {
        Self {
            binary_path: crate::config::CLOUD_HYPERVISOR_BINARY.to_string(),
        }
    }

    pub async fn start(
        &self,
        config: VmmConfig,
        process_manager: &ProcessManager,
    ) -> Result<VmmStartResult, String> {
        // Detect if we're booting from an ISO
        let has_iso = config.disks.iter().any(|d| {
            d.path.to_lowercase().ends_with(".iso")
        });

        // Build cloud-hypervisor command line arguments
        let mut args = vec![
            format!("--cpus boot={}", config.cpus),
            format!("--memory size={}M", config.memory_mb),
            "--serial tty".to_string(),
            "--console off".to_string(),
            format!("--api-socket /run/ch-{}.sock", config.vm_id),
        ];

        // If we have an ISO, we need UEFI firmware and should not specify a kernel
        if has_iso {
            args.push(format!("--firmware {}", crate::config::UEFI_FIRMWARE_PATH));
        } else {
            // Traditional boot: add kernel if specified
            if let Some(kernel) = &config.kernel {
                args.push(format!("--kernel {}", kernel));
            }
        }

        // Add cmdline if specified (only relevant for direct kernel boot)
        if let Some(cmdline) = &config.cmdline {
            if !has_iso {
                args.push(format!("--cmdline \"{}\"", cmdline));
            }
        }

        // Add disks
        for disk in &config.disks {
            args.push(format!(
                "--disk path={}{}",
                disk.path,
                if disk.readonly { ",readonly=on" } else { "" }
            ));
        }

        // Add network interfaces
        for net in &config.networks {
            args.push(format!("--net tap={},mac={}", net.tap, net.mac));
        }

        crate::log!("vmm", "Executing: {} {}", self.binary_path, args.join(" "));

        // Spawn the cloud-hypervisor process
        let pid = process_manager.spawn_external(self.binary_path.clone(), args, HashMap::new())?;

        Ok(VmmStartResult { pid })
    }

    pub async fn stop(&self, pid: i32, force: bool) -> Result<(), String> {
        let signal = if force { 9 } else { 15 };
        nix::sys::signal::kill(
            nix::unistd::Pid::from_raw(pid),
            nix::sys::signal::Signal::try_from(signal)
                .map_err(|e| format!("Invalid signal: {}", e))?,
        )
        .map_err(|e| format!("Failed to send signal to process: {}", e))
    }
}
