use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::bail;
use tokio::sync::mpsc;

use crate::clients::{NetworkClient, TapDevice};
use crate::disk::{self, DiskUsage};
use crate::hypervisor::{self, DiskConfig, VmStartConfig};
use crate::persistence::{self, DiskConfigPersisted, VmPersisted};
use crate::proto::vm::{
    DiskUsage as ProtoDiskUsage, Hypervisor as HypervisorType, VmConfig, VmInfo, VmState,
};

use super::VmCommand;

const DEFAULT_DISK_SIZE_MB: u64 = 1024;

pub struct VmActor {
    network_client: NetworkClient,
    vms: HashMap<String, VmEntry>,
    pending_restarts: Vec<String>,
    kvm_available: bool,
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
        let disk_usage = disk::get_usage(vm_id).ok().map(|u| u.into());

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
            disk_usage,
        }
    }

    fn to_persisted(&self) -> VmPersisted {
        VmPersisted {
            name: self.config.name.clone(),
            cpus: self.config.cpus,
            memory_mb: self.config.memory_mb,
            kernel: self.config.kernel.clone(),
            initrd: self.config.initrd.clone(),
            cmdline: self.config.cmdline.clone(),
            disks: self
                .config
                .disks
                .iter()
                .map(|d| DiskConfigPersisted {
                    path: d.path.clone(),
                    readonly: d.readonly,
                })
                .collect(),
            hypervisor: self.config.hypervisor,
            root_disk_size_mb: self.config.root_disk_size_mb,
            state: self.state.into(),
            created_at: self.created_at,
            started_at: self.started_at,
            tap_device: self.tap_device.as_ref().map(|t| t.name.clone()),
            mac_address: self.tap_device.as_ref().map(|t| t.mac_address.clone()),
        }
    }

    fn from_persisted(persisted: VmPersisted) -> Self {
        let config = VmConfig {
            name: persisted.name,
            cpus: persisted.cpus,
            memory_mb: persisted.memory_mb,
            kernel: persisted.kernel,
            initrd: persisted.initrd,
            cmdline: persisted.cmdline,
            disks: persisted
                .disks
                .into_iter()
                .map(|d| crate::proto::vm::DiskConfig {
                    path: d.path,
                    readonly: d.readonly,
                })
                .collect(),
            hypervisor: persisted.hypervisor,
            root_disk_size_mb: persisted.root_disk_size_mb,
        };

        let tap_device = match (&persisted.tap_device, &persisted.mac_address) {
            (Some(name), Some(mac)) => Some(TapDevice {
                name: name.clone(),
                mac_address: mac.clone(),
            }),
            _ => None,
        };

        Self {
            config,
            state: VmState::try_from(persisted.state).unwrap_or(VmState::Stopped),
            pid: None,
            tap_device,
            created_at: persisted.created_at,
            started_at: persisted.started_at,
        }
    }
}

impl From<DiskUsage> for ProtoDiskUsage {
    fn from(usage: DiskUsage) -> Self {
        Self {
            used_bytes: usage.used_bytes,
            quota_bytes: usage.quota_bytes,
            usage_percent: usage.usage_percent,
        }
    }
}

impl VmActor {
    pub fn new(network_client: NetworkClient, kvm_available: bool) -> Self {
        let (vms, pending_restarts) = Self::load_persisted_state();
        Self::cleanup_orphaned_disks(&vms);

        Self {
            network_client,
            vms,
            pending_restarts,
            kvm_available,
        }
    }

    fn load_persisted_state() -> (HashMap<String, VmEntry>, Vec<String>) {
        let persisted = persistence::load_vms().unwrap_or_default();
        let mut vms = HashMap::new();
        let mut pending_restarts = Vec::new();

        for (vm_id, persisted_vm) in persisted {
            let was_running = persisted_vm.state == VmState::Running as i32;
            let mut entry = VmEntry::from_persisted(persisted_vm);

            if was_running {
                entry.state = VmState::Stopped;
                entry.tap_device = None;
                if sysconfig::vm().auto_restart {
                    pending_restarts.push(vm_id.clone());
                    kmsg::info!(@ "vmd", "VM {} was running, will restart", entry.config.name);
                } else {
                    kmsg::info!(@ "vmd", "VM {} was running, but auto-restart is disabled", entry.config.name);
                }
            }

            vms.insert(vm_id, entry);
        }

        (vms, pending_restarts)
    }

    fn cleanup_orphaned_disks(vms: &HashMap<String, VmEntry>) {
        let disk_vms = disk::list_subvolumes().unwrap_or_default();

        for vm_id in disk_vms {
            if !vms.contains_key(&vm_id) {
                kmsg::warn!(@ "vmd", "Cleaning up orphaned disk: {}", vm_id);
                if let Err(e) = disk::delete_subvolume(&vm_id) {
                    kmsg::error!(@ "vmd", "Failed to delete orphaned disk {}: {}", vm_id, e);
                }
            }
        }
    }

    pub async fn run(&mut self, mut cmd_rx: mpsc::Receiver<VmCommand>) {
        self.process_pending_restarts().await;

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
                    vm_id,
                    reply,
                } => {
                    let result = self
                        .handle_upload_file(&filename, &data, vm_id.as_deref())
                        .await;
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

    async fn process_pending_restarts(&mut self) {
        let restarts = std::mem::take(&mut self.pending_restarts);

        for vm_id in restarts {
            kmsg::info!(@ "vmd", "Auto-restarting VM {}", vm_id);
            if let Err(e) = self.handle_start(&vm_id).await {
                kmsg::error!(@ "vmd", "Failed to auto-restart VM {}: {}", vm_id, e);
            }
        }
    }

    async fn handle_create(&mut self, config: VmConfig) -> anyhow::Result<String> {
        let vm_id = uuid::Uuid::new_v4().to_string();

        kmsg::info!(@ "vmd", "Creating VM {} ({})", config.name, vm_id);

        let size_mb = if config.root_disk_size_mb == 0 {
            DEFAULT_DISK_SIZE_MB
        } else {
            config.root_disk_size_mb
        };
        let size_bytes = size_mb * 1024 * 1024;

        disk::create_subvolume(&vm_id)?;
        disk::set_quota(&vm_id, size_bytes)?;
        disk::create_raw_image(&vm_id, size_bytes)?;

        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0);

        let mut config = config;
        if config.root_disk_size_mb == 0 {
            config.root_disk_size_mb = DEFAULT_DISK_SIZE_MB;
        }

        let entry = VmEntry {
            config,
            state: VmState::Created,
            pid: None,
            tap_device: None,
            created_at: now,
            started_at: None,
        };

        persistence::save_vm(&vm_id, &entry.to_persisted())?;
        self.vms.insert(vm_id.clone(), entry);

        Ok(vm_id)
    }

    async fn handle_start(&mut self, vm_id: &str) -> anyhow::Result<()> {
        if !self.kvm_available {
            bail!("KVM is not available: /dev/kvm is missing or inaccessible, cannot start VMs");
        }

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

        let vm_data_dir = PathBuf::from(disk::DATA_DIR).join(vm_id);
        let serial_log_path = vm_data_dir.join("serial.log");

        let kernel_path = resolve_boot_asset(&vm_data_dir, "kernel")?;
        let initrd_path = resolve_boot_asset(&vm_data_dir, "initrd").ok();

        let mut resolved_disks = Vec::new();
        for (i, d) in entry.config.disks.iter().enumerate() {
            let convention_name = format!("disk{}", i);
            let resolved_path = resolve_boot_asset(&vm_data_dir, &convention_name)?;
            resolved_disks.push(DiskConfig {
                path: resolved_path,
                readonly: d.readonly,
            });
        }

        let start_config = VmStartConfig {
            vm_id: vm_id.to_string(),
            cpus: entry.config.cpus,
            memory_mb: entry.config.memory_mb,
            kernel: kernel_path,
            initrd: initrd_path,
            cmdline: entry.config.cmdline.clone(),
            disks: resolved_disks,
            tap_device: tap.name.clone(),
            mac_address: tap.mac_address.clone(),
            serial_log_path,
            persistent_disk: Some(disk::get_image_path(vm_id)),
        };

        match hypervisor.start(&start_config).await {
            Ok(process) => {
                let now = SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .map(|d| d.as_secs() as i64)
                    .unwrap_or(0);

                entry.pid = Some(process.pid);
                entry.state = VmState::Running;
                entry.started_at = Some(now);

                persistence::save_vm(vm_id, &entry.to_persisted())?;

                kmsg::info!(@ "vmd", "VM {} started with PID {}", entry.config.name, process.pid);
                Ok(())
            }
            Err(e) => {
                kmsg::error!(@ "vmd", "Failed to start VM {}: {}", entry.config.name, e);

                if let Some(tap) = entry.tap_device.take()
                    && let Err(tap_err) = self.network_client.delete_tap(&tap.name).await
                {
                    kmsg::warn!(@ "vmd", "Failed to cleanup TAP device {}: {}", tap.name, tap_err);
                }

                entry.state = VmState::Created;
                persistence::save_vm(vm_id, &entry.to_persisted())?;

                Err(e)
            }
        }
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

        persistence::save_vm(vm_id, &entry.to_persisted())?;

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

        if let Err(e) = disk::delete_subvolume(vm_id) {
            kmsg::warn!(@ "vmd", "Failed to delete disk subvolume: {}", e);
        }

        if let Err(e) = persistence::delete_vm(vm_id) {
            kmsg::warn!(@ "vmd", "Failed to delete VM state file: {}", e);
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

    async fn handle_upload_file(
        &self,
        filename: &str,
        data: &[u8],
        vm_id: Option<&str>,
    ) -> anyhow::Result<String> {
        let vm_id = vm_id.ok_or_else(|| anyhow::anyhow!("vm_id is required for file uploads"))?;

        let safe_filename = filename.replace(['/', '\\', '\0'], "_");

        let vm_dir = PathBuf::from(disk::DATA_DIR).join(vm_id);
        if !vm_dir.exists() {
            anyhow::bail!("VM directory not found: {}. Create the VM first.", vm_id);
        }
        let path = vm_dir.join(&safe_filename);

        tokio::fs::write(&path, data).await?;

        kmsg::info!(@ "vmd", "Uploaded file {} ({} bytes)", path.display(), data.len());

        Ok(path.to_string_lossy().to_string())
    }

    async fn handle_get_serial_log(&self, vm_id: &str, tail_lines: i64) -> anyhow::Result<String> {
        let _ = self
            .vms
            .get(vm_id)
            .ok_or_else(|| anyhow::anyhow!("VM not found: {}", vm_id))?;

        let log_path = PathBuf::from(disk::DATA_DIR).join(vm_id).join("serial.log");

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

fn resolve_boot_asset(vm_data_dir: &Path, convention_name: &str) -> anyhow::Result<PathBuf> {
    let path = vm_data_dir.join(convention_name);
    if path.exists() {
        Ok(path)
    } else {
        anyhow::bail!("{} not found: {}", convention_name, path.display())
    }
}
