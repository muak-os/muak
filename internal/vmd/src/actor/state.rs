use std::collections::HashMap;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use tokio::sync::mpsc;

use crate::clients::{NetworkClient, TapDevice};
use crate::hypervisor::{self, DiskConfig, VmStartConfig};
use crate::proto::vm::{Hypervisor as HypervisorType, VmConfig, VmInfo, VmState};

use super::VmCommand;

const VM_DATA_DIR: &str = "/run/vmd";
const UPLOAD_DIR: &str = "/run/vmd/uploads";

pub struct VmActor {
    network_client: NetworkClient,
    vms: HashMap<String, VmEntry>,
}

struct VmEntry {
    config: VmConfig,
    state: VmState,
    pid: Option<u32>,
    tap_device: Option<TapDevice>,
    created_at: i64,
    started_at: Option<i64>,
}

impl VmEntry {
    fn to_info(&self, vm_id: &str) -> VmInfo {
        VmInfo {
            vm_id: vm_id.to_string(),
            name: self.config.name.clone(),
            state: self.state.into(),
            config: Some(self.config.clone()),
            pid: self.pid.map(|p| p as i32).unwrap_or(0),
            created_at: self.created_at,
            started_at: self.started_at.unwrap_or(0),
            tap_device: self
                .tap_device
                .as_ref()
                .map(|t| t.name.clone())
                .unwrap_or_default(),
            mac_address: self
                .tap_device
                .as_ref()
                .map(|t| t.mac_address.clone())
                .unwrap_or_default(),
        }
    }
}

impl VmActor {
    pub fn new(network_client: NetworkClient) -> Self {
        Self {
            network_client,
            vms: HashMap::new(),
        }
    }

    pub async fn run(&mut self, mut cmd_rx: mpsc::Receiver<VmCommand>) {
        let _ = tokio::fs::create_dir_all(VM_DATA_DIR).await;
        let _ = tokio::fs::create_dir_all(UPLOAD_DIR).await;

        while let Some(cmd) = cmd_rx.recv().await {
            match cmd {
                VmCommand::Create { config, reply } => {
                    let result = self.handle_create(config).await;
                    let _ = reply.send(result);
                }
                VmCommand::Start { vm_id, reply } => {
                    let result = self.handle_start(&vm_id).await;
                    let _ = reply.send(result);
                }
                VmCommand::Stop {
                    vm_id,
                    force,
                    reply,
                } => {
                    let result = self.handle_stop(&vm_id, force).await;
                    let _ = reply.send(result);
                }
                VmCommand::Delete { vm_id, reply } => {
                    let result = self.handle_delete(&vm_id).await;
                    let _ = reply.send(result);
                }
                VmCommand::Get { vm_id, reply } => {
                    let result = self.handle_get(&vm_id);
                    let _ = reply.send(result);
                }
                VmCommand::List { reply } => {
                    let result = self.handle_list();
                    let _ = reply.send(result);
                }
                VmCommand::UploadFile {
                    filename,
                    data,
                    reply,
                } => {
                    let result = self.handle_upload_file(&filename, &data).await;
                    let _ = reply.send(result);
                }
                VmCommand::GetSerialLog {
                    vm_id,
                    tail_lines,
                    reply,
                } => {
                    let result = self.handle_get_serial_log(&vm_id, tail_lines).await;
                    let _ = reply.send(result);
                }
            }
        }
    }

    async fn handle_create(&mut self, config: VmConfig) -> anyhow::Result<String> {
        let vm_id = uuid::Uuid::new_v4().to_string();

        kmsg::info!(@ "vmd", "Creating VM {} ({})", config.name, vm_id);

        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0);

        let entry = VmEntry {
            config,
            state: VmState::Created,
            pid: None,
            tap_device: None,
            created_at: now,
            started_at: None,
        };

        self.vms.insert(vm_id.clone(), entry);

        let vm_dir = PathBuf::from(VM_DATA_DIR).join(&vm_id);
        tokio::fs::create_dir_all(&vm_dir).await?;

        Ok(vm_id)
    }

    async fn handle_start(&mut self, vm_id: &str) -> anyhow::Result<()> {
        let entry = self
            .vms
            .get_mut(vm_id)
            .ok_or_else(|| anyhow::anyhow!("VM not found: {}", vm_id))?;

        if entry.state == VmState::Running {
            anyhow::bail!("VM is already running");
        }

        kmsg::info!(@ "vmd", "Starting VM {} ({})", entry.config.name, vm_id);
        entry.state = VmState::Starting;

        let tap = self.network_client.create_tap(vm_id, None).await?;
        kmsg::info!(@ "vmd", "Created TAP device {} with MAC {}", tap.name, tap.mac_address);

        entry.tap_device = Some(tap.clone());

        let hypervisor_type =
            HypervisorType::try_from(entry.config.hypervisor).unwrap_or(HypervisorType::Qemu);
        let hypervisor = hypervisor::create_hypervisor(hypervisor_type);

        let vm_dir = PathBuf::from(VM_DATA_DIR).join(vm_id);
        let serial_log_path = vm_dir.join("serial.log");

        let start_config = VmStartConfig {
            vm_id: vm_id.to_string(),
            cpus: entry.config.cpus,
            memory_mb: entry.config.memory_mb,
            kernel: PathBuf::from(&entry.config.kernel),
            initrd: if entry.config.initrd.is_empty() {
                None
            } else {
                Some(PathBuf::from(&entry.config.initrd))
            },
            cmdline: entry.config.cmdline.clone(),
            disks: entry
                .config
                .disks
                .iter()
                .map(|d| DiskConfig {
                    path: PathBuf::from(&d.path),
                    readonly: d.readonly,
                })
                .collect(),
            tap_device: tap.name.clone(),
            mac_address: tap.mac_address.clone(),
            serial_log_path,
        };

        let process = hypervisor.start(&start_config).await?;

        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0);

        entry.pid = Some(process.pid);
        entry.state = VmState::Running;
        entry.started_at = Some(now);

        kmsg::info!(@ "vmd", "VM {} started with PID {}", entry.config.name, process.pid);

        Ok(())
    }

    async fn handle_stop(&mut self, vm_id: &str, force: bool) -> anyhow::Result<()> {
        let entry = self
            .vms
            .get_mut(vm_id)
            .ok_or_else(|| anyhow::anyhow!("VM not found: {}", vm_id))?;

        if entry.state != VmState::Running {
            anyhow::bail!("VM is not running");
        }

        kmsg::info!(@ "vmd", "Stopping VM {} (force={})", entry.config.name, force);
        entry.state = VmState::Stopping;

        if let Some(pid) = entry.pid {
            let hypervisor_type =
                HypervisorType::try_from(entry.config.hypervisor).unwrap_or(HypervisorType::Qemu);
            let hypervisor = hypervisor::create_hypervisor(hypervisor_type);
            hypervisor.stop(pid, force).await?;
        }

        if let Some(tap) = entry.tap_device.take()
            && let Err(e) = self.network_client.delete_tap(&tap.name).await
        {
            kmsg::warn!(@ "vmd", "Failed to delete TAP device {}: {}", tap.name, e);
        }

        entry.state = VmState::Stopped;
        entry.pid = None;

        Ok(())
    }

    async fn handle_delete(&mut self, vm_id: &str) -> anyhow::Result<()> {
        let entry = self
            .vms
            .get(vm_id)
            .ok_or_else(|| anyhow::anyhow!("VM not found: {}", vm_id))?;

        if entry.state == VmState::Running {
            anyhow::bail!("Cannot delete running VM");
        }

        kmsg::info!(@ "vmd", "Deleting VM {}", vm_id);

        let vm_dir = PathBuf::from(VM_DATA_DIR).join(vm_id);
        if vm_dir.exists() {
            tokio::fs::remove_dir_all(&vm_dir).await?;
        }

        self.vms.remove(vm_id);

        Ok(())
    }

    fn handle_get(&self, vm_id: &str) -> anyhow::Result<VmInfo> {
        let entry = self
            .vms
            .get(vm_id)
            .ok_or_else(|| anyhow::anyhow!("VM not found: {}", vm_id))?;

        Ok(entry.to_info(vm_id))
    }

    fn handle_list(&self) -> anyhow::Result<Vec<VmInfo>> {
        let vms = self
            .vms
            .iter()
            .map(|(id, entry)| entry.to_info(id))
            .collect();

        Ok(vms)
    }

    async fn handle_upload_file(&self, filename: &str, data: &[u8]) -> anyhow::Result<String> {
        let safe_filename = filename.replace(['/', '\\', '\0'], "_");
        let path = PathBuf::from(UPLOAD_DIR).join(&safe_filename);

        tokio::fs::write(&path, data).await?;

        kmsg::info!(@ "vmd", "Uploaded file {} ({} bytes)", safe_filename, data.len());

        Ok(path.to_string_lossy().to_string())
    }

    async fn handle_get_serial_log(&self, vm_id: &str, tail_lines: i64) -> anyhow::Result<String> {
        let _ = self
            .vms
            .get(vm_id)
            .ok_or_else(|| anyhow::anyhow!("VM not found: {}", vm_id))?;

        let log_path = PathBuf::from(VM_DATA_DIR).join(vm_id).join("serial.log");

        if !log_path.exists() {
            return Ok(String::new());
        }

        let content = tokio::fs::read_to_string(&log_path).await?;

        if tail_lines > 0 {
            let lines: Vec<&str> = content.lines().collect();
            let start = lines.len().saturating_sub(tail_lines as usize);
            Ok(lines[start..].join("\n"))
        } else {
            Ok(content)
        }
    }
}
