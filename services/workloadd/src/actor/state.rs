extern crate alloc;

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::bail;
use btrfs::quota::{DiskUsage, get_usage, set};
use btrfs::subvolume::{create, delete, list};
use netlib::mac::{format as format_mac, generate as generate_mac};
use netlib::tap::{remove as net_remove_tap, setup_on_bridge};
use tokio::fs::{read_to_string, write};
use tokio::sync::mpsc;

use super::VmCommand;
use crate::disk;
use crate::disk::image::{create_raw, get_path};
use crate::hypervisor::{self, DiskConfig, VmStartConfig};
use crate::persistence::state::{DiskConfigPersisted, VmPersisted, delete_vm, load_vms, save_vm};
use crate::proto::vm::{
    DiskConfig as ProtoDiskConfig, DiskUsage as ProtoDiskUsage, Hypervisor as HypervisorType,
    VmConfig, VmInfo, VmState,
};

const DEFAULT_DISK_SIZE_MB: u64 = 1024;

#[derive(Clone)]
struct TapDevice {
    name: String,
    mac_address: String,
}

pub struct VmActor {
    netlink_handle: rtnetlink::Handle,
    bridge_name: String,
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
        let disk_usage = get_usage(vm_id, disk::DATA_DIR).ok().map(Into::into);

        VmInfo {
            vm_id: vm_id.to_owned(),
            name: self.config.name.clone(),
            state: self.state.into(),
            config: Some(self.config.clone()),
            pid: self.pid.map_or(0, |pid| i32::try_from(pid).unwrap_or(0)),
            created_at: self.created_at,
            started_at: self.started_at.unwrap_or(0),
            tap_device: self
                .tap_device
                .as_ref()
                .map(|tap| tap.name.clone())
                .unwrap_or_default(),
            mac_address: self
                .tap_device
                .as_ref()
                .map(|tap| tap.mac_address.clone())
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
                .map(|disk| DiskConfigPersisted {
                    path: disk.path.clone(),
                    readonly: disk.readonly,
                })
                .collect(),
            hypervisor: self.config.hypervisor,
            root_disk_size_mb: self.config.root_disk_size_mb,
            state: self.state.into(),
            created_at: self.created_at,
            started_at: self.started_at,
            tap_device: self.tap_device.as_ref().map(|tap| tap.name.clone()),
            mac_address: self.tap_device.as_ref().map(|tap| tap.mac_address.clone()),
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
                .map(|disk| ProtoDiskConfig {
                    path: disk.path,
                    readonly: disk.readonly,
                })
                .collect(),
            hypervisor: persisted.hypervisor,
            root_disk_size_mb: persisted.root_disk_size_mb,
        };

        let tap_device = match (
            persisted.tap_device.as_deref(),
            persisted.mac_address.as_deref(),
        ) {
            (Some(name), Some(mac)) => Some(TapDevice {
                name: name.to_owned(),
                mac_address: mac.to_owned(),
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
            usage_percent: f32::from(usage.usage_percent),
        }
    }
}

fn reset_if_running(
    entry: &mut VmEntry,
    was_running: bool,
    vm_id: &str,
    pending_restarts: &mut Vec<String>,
) {
    if !was_running {
        return;
    }
    entry.state = VmState::Stopped;
    entry.tap_device = None;
    if config::vm().auto_restart {
        pending_restarts.push(vm_id.to_owned());
        println!("VM {} was running, will restart", entry.config.name);
    } else {
        println!(
            "VM {} was running, but auto-restart is disabled",
            entry.config.name
        );
    }
}

impl VmActor {
    pub fn new(
        netlink_handle: rtnetlink::Handle,
        bridge_name: String,
        kvm_available: bool,
    ) -> Self {
        let (vms, pending_restarts) = Self::load_persisted_state();
        Self::cleanup_orphaned_disks(&vms);

        Self {
            netlink_handle,
            bridge_name,
            vms,
            pending_restarts,
            kvm_available,
        }
    }

    fn load_persisted_state() -> (HashMap<String, VmEntry>, Vec<String>) {
        let mut persisted: Vec<(String, VmPersisted)> =
            load_vms().unwrap_or_default().into_iter().collect();
        persisted.sort_by(|left, right| left.0.cmp(&right.0));

        let mut vms = HashMap::new();
        let mut pending_restarts = Vec::new();

        for (vm_id, persisted_vm) in persisted {
            let was_running = VmState::try_from(persisted_vm.state).ok() == Some(VmState::Running);
            let mut entry = VmEntry::from_persisted(persisted_vm);
            reset_if_running(&mut entry, was_running, &vm_id, &mut pending_restarts);
            vms.insert(vm_id, entry);
        }

        (vms, pending_restarts)
    }

    fn cleanup_orphaned_disks(vms: &HashMap<String, VmEntry>) {
        list(disk::DATA_DIR)
            .unwrap_or_default()
            .into_iter()
            .filter(|vm_id| !vms.contains_key(vm_id))
            .for_each(|vm_id| delete_orphaned_disk(&vm_id));
    }

    pub async fn run(&mut self, mut cmd_rx: mpsc::Receiver<VmCommand>) {
        self.process_pending_restarts().await;

        while let Some(cmd) = cmd_rx.recv().await {
            self.dispatch(cmd).await;
        }
    }

    async fn dispatch(&mut self, cmd: VmCommand) {
        match cmd {
            VmCommand::Create { config, reply } => {
                let _reply = reply.send(self.handle_create(config));
            }
            VmCommand::Start { vm_id, reply } => {
                let _reply = reply.send(self.handle_start(&vm_id).await);
            }
            VmCommand::Stop {
                vm_id,
                force,
                reply,
            } => {
                let _reply = reply.send(self.handle_stop(&vm_id, force).await);
            }
            VmCommand::Delete { vm_id, reply } => {
                let _reply = reply.send(self.handle_delete(&vm_id));
            }
            VmCommand::Get { vm_id, reply } => {
                let _reply = reply.send(self.handle_get(&vm_id));
            }
            VmCommand::List { reply } => {
                let _reply = reply.send(Ok(self.handle_list()));
            }
            VmCommand::UploadFile {
                filename,
                data,
                vm_id,
                reply,
            } => {
                let _reply = reply.send(
                    self.handle_upload_file(&filename, &data, vm_id.as_deref())
                        .await,
                );
            }
            VmCommand::GetSerialLog {
                vm_id,
                tail_lines,
                reply,
            } => {
                let _reply = reply.send(self.handle_get_serial_log(&vm_id, tail_lines).await);
            }
        }
    }

    async fn process_pending_restarts(&mut self) {
        for vm_id in core::mem::take(&mut self.pending_restarts) {
            self.auto_restart_vm(&vm_id).await;
        }
    }

    async fn auto_restart_vm(&mut self, vm_id: &str) {
        println!("Auto-restarting VM {vm_id}");
        if let Err(e) = self.handle_start(vm_id).await {
            eprintln!("Failed to auto-restart VM {vm_id}: {e}");
        }
    }

    fn handle_create(&mut self, config: VmConfig) -> anyhow::Result<String> {
        let vm_id = uuid::Uuid::new_v4().to_string();

        println!("Creating VM {} ({})", config.name, vm_id);

        let size_mb = if config.root_disk_size_mb == 0 {
            DEFAULT_DISK_SIZE_MB
        } else {
            config.root_disk_size_mb
        };
        let size_bytes = size_mb.saturating_mul(1024 * 1024);

        create(&vm_id, disk::DATA_DIR)?;
        set(&vm_id, size_bytes, disk::DATA_DIR)?;
        create_raw(&vm_id, size_bytes)?;

        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |duration| i64::try_from(duration.as_secs()).unwrap_or(0));

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

        save_vm(&vm_id, &entry.to_persisted())?;
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
            .ok_or_else(|| anyhow::anyhow!("VM not found: {vm_id}"))?;

        if entry.state == VmState::Running {
            anyhow::bail!("VM is already running");
        }

        println!("Starting VM {} ({})", entry.config.name, vm_id);
        entry.state = VmState::Starting;

        let tap_name = format!(
            "tap-{}",
            vm_id.get(..8.min(vm_id.len())).unwrap_or_default()
        );
        setup_on_bridge(&self.netlink_handle, &tap_name, &self.bridge_name).await?;
        let mac = generate_mac(vm_id);
        let tap = TapDevice {
            name: tap_name,
            mac_address: format_mac(&mac),
        };
        println!(
            "Created TAP device {} with MAC {}",
            tap.name, tap.mac_address
        );

        entry.tap_device = Some(tap.clone());

        let hypervisor_type =
            HypervisorType::try_from(entry.config.hypervisor).unwrap_or(HypervisorType::Qemu);
        let hypervisor = hypervisor::create_hypervisor(hypervisor_type);

        let vm_data_dir = PathBuf::from(disk::DATA_DIR).join(vm_id);
        let serial_log_path = vm_data_dir.join("serial.log");

        let kernel_path = resolve_boot_asset(&vm_data_dir, "kernel")?;
        let initrd_path = resolve_boot_asset(&vm_data_dir, "initrd").ok();

        let mut resolved_disks = Vec::new();
        for (disk_idx, disk) in entry.config.disks.iter().enumerate() {
            let convention_name = format!("disk{disk_idx}");
            let resolved_path = resolve_boot_asset(&vm_data_dir, &convention_name)?;
            resolved_disks.push(DiskConfig {
                path: resolved_path,
                readonly: disk.readonly,
            });
        }

        let start_config = VmStartConfig {
            vm_id: vm_id.to_owned(),
            cpus: entry.config.cpus,
            memory_mb: entry.config.memory_mb,
            kernel: kernel_path,
            initrd: initrd_path,
            cmdline: entry.config.cmdline.clone(),
            disks: resolved_disks,
            tap_device: tap.name.clone(),
            mac_address: tap.mac_address.clone(),
            serial_log_path,
            persistent_disk: Some(get_path(vm_id)),
        };

        match hypervisor.start(&start_config).await {
            Ok(process) => {
                let now = SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .map_or(0, |duration| i64::try_from(duration.as_secs()).unwrap_or(0));

                entry.pid = Some(process.pid);
                entry.state = VmState::Running;
                entry.started_at = Some(now);

                save_vm(vm_id, &entry.to_persisted())?;

                println!("VM {} started with PID {}", entry.config.name, process.pid);
                Ok(())
            }
            Err(e) => {
                eprintln!("Failed to start VM {}: {}", entry.config.name, e);
                let tap_name = entry.tap_device.take().map(|tap| tap.name);
                entry.state = VmState::Created;
                save_vm(vm_id, &entry.to_persisted())?;
                cleanup_tap_on_err(&self.netlink_handle, tap_name).await;
                Err(e)
            }
        }
    }

    async fn handle_stop(&mut self, vm_id: &str, force: bool) -> anyhow::Result<()> {
        let entry = self
            .vms
            .get_mut(vm_id)
            .ok_or_else(|| anyhow::anyhow!("VM not found: {vm_id}"))?;

        if entry.state != VmState::Running {
            anyhow::bail!("VM is not running");
        }

        println!("Stopping VM {} (force={})", entry.config.name, force);
        entry.state = VmState::Stopping;

        if let Some(pid) = entry.pid {
            let hypervisor_type =
                HypervisorType::try_from(entry.config.hypervisor).unwrap_or(HypervisorType::Qemu);
            let hypervisor = hypervisor::create_hypervisor(hypervisor_type);
            hypervisor.stop(pid, force)?;
        }

        if let Some(tap) = entry.tap_device.take() {
            let tap_name = tap.name;
            entry.state = VmState::Stopped;
            entry.pid = None;
            save_vm(vm_id, &entry.to_persisted())?;
            remove_tap(&self.netlink_handle, &tap_name).await;
            return Ok(());
        }

        entry.state = VmState::Stopped;
        entry.pid = None;

        save_vm(vm_id, &entry.to_persisted())?;

        Ok(())
    }

    fn handle_delete(&mut self, vm_id: &str) -> anyhow::Result<()> {
        let entry = self
            .vms
            .get(vm_id)
            .ok_or_else(|| anyhow::anyhow!("VM not found: {vm_id}"))?;

        if entry.state == VmState::Running {
            anyhow::bail!("Cannot delete running VM");
        }

        println!("Deleting VM {vm_id}");

        if let Err(e) = delete(vm_id, disk::DATA_DIR) {
            eprintln!("Failed to delete disk subvolume: {e}");
        }

        if let Err(e) = delete_vm(vm_id) {
            eprintln!("Failed to delete VM state file: {e}");
        }

        self.vms.remove(vm_id);

        Ok(())
    }

    fn handle_get(&self, vm_id: &str) -> anyhow::Result<VmInfo> {
        let entry = self
            .vms
            .get(vm_id)
            .ok_or_else(|| anyhow::anyhow!("VM not found: {vm_id}"))?;

        Ok(entry.to_info(vm_id))
    }

    fn handle_list(&self) -> Vec<VmInfo> {
        self.vms
            .iter()
            .map(|(id, entry)| entry.to_info(id))
            .collect()
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
            anyhow::bail!("VM directory not found: {vm_id}. Create the VM first.");
        }
        let path = vm_dir.join(&safe_filename);

        write(&path, data).await?;

        println!("Uploaded file {} ({} bytes)", path.display(), data.len());

        Ok(path.to_string_lossy().to_string())
    }

    async fn handle_get_serial_log(&self, vm_id: &str, tail_lines: i64) -> anyhow::Result<String> {
        self.vms
            .get(vm_id)
            .ok_or_else(|| anyhow::anyhow!("VM not found: {vm_id}"))?;

        let log_path = PathBuf::from(disk::DATA_DIR).join(vm_id).join("serial.log");

        if !log_path.exists() {
            return Ok(String::new());
        }

        let content = read_to_string(&log_path).await?;

        if tail_lines > 0 {
            let lines: Vec<&str> = content.lines().collect();
            let start = lines
                .len()
                .saturating_sub(usize::try_from(tail_lines).unwrap_or(0));
            Ok(lines.get(start..).unwrap_or_default().join("\n"))
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

async fn remove_tap(handle: &rtnetlink::Handle, tap_name: &str) {
    if let Err(e) = net_remove_tap(handle, tap_name).await {
        eprintln!("Failed to delete TAP device {tap_name}: {e}");
    }
}

async fn cleanup_tap_on_err(handle: &rtnetlink::Handle, tap_name: Option<String>) {
    if let Some(name) = tap_name {
        remove_tap(handle, &name).await;
    }
}

fn delete_orphaned_disk(vm_id: &str) {
    kmsg::warn!(@ "workloadd", "Cleaning up orphaned disk: {}", vm_id);
    if let Err(e) = delete(vm_id, disk::DATA_DIR) {
        kmsg::error!(@ "workloadd", "Failed to delete orphaned disk {}: {}", vm_id, e);
    }
}
