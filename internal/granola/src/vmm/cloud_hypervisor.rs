use super::{VmmConfig, VmmStartResult};
use crate::process::ProcessManager;

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
        let boot_mode = if config.kernel.is_some() {
            "direct kernel boot"
        } else {
            "UEFI firmware boot (edk2 CLOUDHV)"
        };

        kmsg::info!(@ "vmm", "Starting cloud-hypervisor v42.0 with {}", boot_mode);

        // Build cloud-hypervisor command line arguments
        let mut args = vec![
            "--cpus".to_string(),
            format!("boot={}", config.cpus),
            "--memory".to_string(),
            format!("size={}M", config.memory_mb),
            "--api-socket".to_string(),
            format!("/run/ch-{}.sock", config.vm_id),
            "--serial".to_string(),
            "tty".to_string(),
        ];

        // Choose boot mode: direct kernel or UEFI firmware
        if let Some(kernel_path) = &config.kernel {
            // Direct kernel boot mode
            kmsg::info!(
                @ "vmm",
                "Using direct kernel boot with kernel: {}",
                kernel_path
            );
            args.push("--kernel".to_string());
            args.push(kernel_path.clone());

            // Add initrd if provided
            if let Some(initrd_path) = &config.initrd {
                kmsg::info!(@ "vmm", "Using initrd: {}", initrd_path);
                args.push("--initramfs".to_string());
                args.push(initrd_path.clone());
            }

            // Add kernel command line if provided
            if let Some(cmdline) = &config.cmdline {
                kmsg::info!(@ "vmm", "Kernel cmdline: {}", cmdline);
                args.push("--cmdline".to_string());
                args.push(cmdline.clone());
            }
        } else {
            // UEFI firmware boot mode (default)
            kmsg::info!(
                @ "vmm",
                "Using UEFI firmware: {}",
                crate::config::UEFI_FIRMWARE_PATH
            );
            args.push("--kernel".to_string());
            args.push(crate::config::UEFI_FIRMWARE_PATH.to_string());
        }

        // Add disks
        for disk in &config.disks {
            args.push("--disk".to_string());
            args.push(format!(
                "path={}{}",
                disk.path,
                if disk.readonly { ",readonly=on" } else { "" }
            ));
        }

        // Add network interfaces
        for net in &config.networks {
            args.push("--net".to_string());
            args.push(format!("tap={},mac={}", net.tap, net.mac));
        }

        kmsg::info!(@ "vmm", "Executing: {} {}", self.binary_path, args.join(" "));

        // Spawn the cloud-hypervisor process with stdout/stderr redirected to serial log
        let log_path = format!("/run/{}-serial.log", config.vm_id);
        let pid = process_manager.spawn_external_with_redirect(
            self.binary_path.clone(),
            args,
            Some(log_path.clone()),
            Some(log_path),
        )?;

        Ok(VmmStartResult { pid })
    }

    #[allow(dead_code)]
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
