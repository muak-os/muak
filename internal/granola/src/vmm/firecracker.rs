use super::{VmmConfig, VmmStartResult};
use crate::process::ProcessManager;
use serde::{Deserialize, Serialize};

pub struct FirecrackerBackend {
    binary_path: String,
}

#[derive(Debug, Serialize, Deserialize)]
struct BootSource {
    kernel_image_path: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    boot_args: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
struct Drive {
    drive_id: String,
    path_on_host: String,
    is_root_device: bool,
    is_read_only: bool,
}

#[derive(Debug, Serialize, Deserialize)]
struct NetworkInterface {
    iface_id: String,
    guest_mac: String,
    host_dev_name: String,
}

#[derive(Debug, Serialize, Deserialize)]
struct MachineConfig {
    vcpu_count: i32,
    mem_size_mib: i64,
}

impl FirecrackerBackend {
    pub fn new() -> Self {
        Self {
            binary_path: crate::config::FIRECRACKER_BINARY.to_string(),
        }
    }

    pub async fn start(
        &self,
        config: VmmConfig,
        process_manager: &ProcessManager,
    ) -> Result<VmmStartResult, String> {
        let socket_path = format!("/run/firecracker-{}.sock", config.vm_id);

        // Start Firecracker with API socket
        let args = vec!["--api-sock".to_string(), socket_path.clone()];

        kmsg::info!(
            @ "vmm",
            "Starting Firecracker: {} {}",
            self.binary_path,
            args.join(" ")
        );

        let pid = process_manager.spawn_external(self.binary_path.clone(), args)?;

        kmsg::info!(@ "vmm", "Firecracker process started with PID {}", pid);

        // Wait a bit for the API socket to be ready
        tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;

        // Configure VM via API
        if let Err(e) = self.configure_vm(&socket_path, &config).await {
            // If configuration fails, kill the process
            let _ = nix::sys::signal::kill(
                nix::unistd::Pid::from_raw(pid),
                nix::sys::signal::Signal::SIGKILL,
            );
            return Err(format!("Failed to configure Firecracker VM: {}", e));
        }

        // Start the VM
        if let Err(e) = self.start_vm(&socket_path).await {
            // If start fails, kill the process
            let _ = nix::sys::signal::kill(
                nix::unistd::Pid::from_raw(pid),
                nix::sys::signal::Signal::SIGKILL,
            );
            return Err(format!("Failed to start Firecracker VM: {}", e));
        }

        kmsg::info!(@ "vmm", "Firecracker VM configured and started");

        Ok(VmmStartResult { pid })
    }

    #[allow(dead_code)]
    pub async fn stop(&self, pid: i32, force: bool) -> Result<(), String> {
        let signal = if force {
            nix::sys::signal::Signal::SIGKILL
        } else {
            nix::sys::signal::Signal::SIGTERM
        };

        nix::sys::signal::kill(nix::unistd::Pid::from_raw(pid), signal)
            .map_err(|e| format!("Failed to send signal to process: {}", e))
    }
}

impl FirecrackerBackend {
    async fn configure_vm(&self, socket_path: &str, config: &VmmConfig) -> Result<(), String> {
        // 1. Set boot source
        // Use Firecracker's default kernel if no kernel is specified
        let kernel_path = config
            .kernel
            .clone()
            .unwrap_or_else(|| crate::config::FIRECRACKER_KERNEL_PATH.to_string());

        // Default boot args for console output and networking
        let default_boot_args = "console=ttyS0 reboot=k panic=1 pci=off ip=dhcp".to_string();
        let boot_args = config.cmdline.clone().or(Some(default_boot_args));

        let boot_source = BootSource {
            kernel_image_path: kernel_path,
            boot_args,
        };
        self.put_api(socket_path, "/boot-source", &boot_source)
            .await?;

        // 2. Set machine config
        let machine_config = MachineConfig {
            vcpu_count: config.cpus,
            mem_size_mib: config.memory_mb,
        };
        self.put_api(socket_path, "/machine-config", &machine_config)
            .await?;

        // 3. Add drives
        // If no disks are specified, use the default rootfs
        if config.disks.is_empty() {
            let drive = Drive {
                drive_id: "rootfs".to_string(),
                path_on_host: crate::config::FIRECRACKER_ROOTFS_PATH.to_string(),
                is_root_device: true,
                is_read_only: false,
            };
            self.put_api(socket_path, "/drives/rootfs", &drive).await?;
        } else {
            for (idx, disk) in config.disks.iter().enumerate() {
                let drive = Drive {
                    drive_id: format!("drive{}", idx),
                    path_on_host: disk.path.clone(),
                    is_root_device: idx == 0,
                    is_read_only: disk.readonly,
                };
                self.put_api(socket_path, &format!("/drives/{}", drive.drive_id), &drive)
                    .await?;
            }
        }

        // 4. Add network interfaces
        for (idx, net) in config.networks.iter().enumerate() {
            let iface = NetworkInterface {
                iface_id: format!("eth{}", idx),
                guest_mac: net.mac.clone(),
                host_dev_name: net.tap.clone(),
            };
            self.put_api(
                socket_path,
                &format!("/network-interfaces/{}", iface.iface_id),
                &iface,
            )
            .await?;
        }

        Ok(())
    }

    async fn start_vm(&self, socket_path: &str) -> Result<(), String> {
        #[derive(Serialize)]
        struct ActionInfo {
            action_type: String,
        }

        let action = ActionInfo {
            action_type: "InstanceStart".to_string(),
        };

        self.put_api(socket_path, "/actions", &action).await
    }

    async fn put_api<T: Serialize>(
        &self,
        socket_path: &str,
        endpoint: &str,
        body: &T,
    ) -> Result<(), String> {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        use tokio::net::UnixStream;

        let mut stream = UnixStream::connect(socket_path)
            .await
            .map_err(|e| format!("Failed to connect to Firecracker API: {}", e))?;

        let body_json =
            serde_json::to_string(body).map_err(|e| format!("Failed to serialize JSON: {}", e))?;

        let request = format!(
            "PUT {} HTTP/1.1\r\n\
             Content-Type: application/json\r\n\
             Content-Length: {}\r\n\
             \r\n\
             {}",
            endpoint,
            body_json.len(),
            body_json
        );

        stream
            .write_all(request.as_bytes())
            .await
            .map_err(|e| format!("Failed to write to API socket: {}", e))?;

        // Read response
        let mut response = String::new();
        stream
            .read_to_string(&mut response)
            .await
            .map_err(|e| format!("Failed to read API response: {}", e))?;

        // Check for HTTP 2xx status
        if !response.starts_with("HTTP/1.1 2") {
            return Err(format!("API request failed: {}", response));
        }

        Ok(())
    }
}
