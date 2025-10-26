use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Vm {
    pub vm_id: String,
    pub name: String,
    pub state: VmState,
    pub config: VmConfig,
    pub pid: Option<i32>,
    pub created_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VmConfig {
    pub cpus: i32,
    pub memory_mb: i64,
    pub kernel: String,
    pub cmdline: Option<String>,
    pub disks: Vec<DiskConfig>,
    pub networks: Vec<NetConfig>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiskConfig {
    pub path: String,
    pub readonly: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NetConfig {
    pub tap: String,
    pub mac: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum VmState {
    Created,
    Starting,
    Running,
    Stopping,
    Stopped,
    Failed(String),
}

impl ToString for VmState {
    fn to_string(&self) -> String {
        match self {
            VmState::Created => "created".to_string(),
            VmState::Starting => "starting".to_string(),
            VmState::Running => "running".to_string(),
            VmState::Stopping => "stopping".to_string(),
            VmState::Stopped => "stopped".to_string(),
            VmState::Failed(e) => format!("failed: {}", e),
        }
    }
}

#[derive(Clone)]
pub struct VmManager {
    vms: Arc<Mutex<HashMap<String, Vm>>>,
    process_manager: crate::process::ProcessManager,
}

impl VmManager {
    pub fn new(process_manager: crate::process::ProcessManager) -> Self {
        Self {
            vms: Arc::new(Mutex::new(HashMap::new())),
            process_manager,
        }
    }

    pub fn create(&self, name: String, config: VmConfig) -> Result<String, String> {
        let vm_id = format!("vm-{}", uuid::Uuid::new_v4());

        let vm = Vm {
            vm_id: vm_id.clone(),
            name,
            state: VmState::Created,
            config,
            pid: None,
            created_at: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("FATAL: system time is before UNIX epoch")
                .as_secs() as i64,
        };

        let mut vms = self
            .vms
            .lock()
            .expect("FATAL: VmManager mutex poisoned - this is a critical PID 1 failure");
        vms.insert(vm_id.clone(), vm);

        Ok(vm_id)
    }

    pub fn start(&self, vm_id: &str) -> Result<(), String> {
        let vm_config;
        let vm_name;

        {
            let mut vms = self
                .vms
                .lock()
                .expect("FATAL: VmManager mutex poisoned - this is a critical PID 1 failure");
            let vm = vms.get_mut(vm_id).ok_or("VM not found")?;

            if vm.state != VmState::Created && vm.state != VmState::Stopped {
                let err_msg = format!("Cannot start VM in state: {}", vm.state.to_string());
                crate::log!("vm", "{}", err_msg);
                return Err(err_msg);
            }

            vm.state = VmState::Starting;
            vm_name = vm.name.clone();
            vm_config = vm.config.clone();
        }

        crate::log!("vm", "Starting VM {} ({})", vm_id, vm_name);

        if !std::path::Path::new(crate::config::CLOUD_HYPERVISOR_BINARY).exists() {
            let err_msg = format!(
                "cloud-hypervisor binary not found at {}",
                crate::config::CLOUD_HYPERVISOR_BINARY
            );

            let mut vms = self
                .vms
                .lock()
                .expect("FATAL: VmManager mutex poisoned - this is a critical PID 1 failure");
            if let Some(vm) = vms.get_mut(vm_id) {
                vm.state = VmState::Failed(err_msg.clone());
            }

            crate::log!("vm", "ERROR: {}", err_msg);
            return Err("cloud-hypervisor extension not installed".to_string());
        }

        for disk in &vm_config.disks {
            if !std::path::Path::new(&disk.path).exists() {
                let err_msg = format!("Disk not found: {}", disk.path);

                let mut vms = self
                    .vms
                    .lock()
                    .expect("FATAL: VmManager mutex poisoned - this is a critical PID 1 failure");
                if let Some(vm) = vms.get_mut(vm_id) {
                    vm.state = VmState::Failed(err_msg.clone());
                }

                crate::log!("vm", "ERROR: {}", err_msg);
                return Err(err_msg);
            }
        }

        let mut args = vec![
            format!("--cpus boot={}", vm_config.cpus),
            format!("--memory size={}M", vm_config.memory_mb),
            "--serial tty".to_string(),
            "--console off".to_string(),
            format!("--api-socket /run/ch-{}.sock", vm_id),
        ];

        for disk in &vm_config.disks {
            args.push(format!(
                "--disk path={}{}",
                disk.path,
                if disk.readonly { ",readonly=on" } else { "" }
            ));
        }

        for net in &vm_config.networks {
            args.push(format!("--net tap={},mac={}", net.tap, net.mac));
        }

        crate::log!(
            "vm",
            "Executing: {} {}",
            crate::config::CLOUD_HYPERVISOR_BINARY,
            args.join(" ")
        );

        let pid = match self.process_manager.spawn_external(
            crate::config::CLOUD_HYPERVISOR_BINARY.to_string(),
            args,
            HashMap::new(),
        ) {
            Ok(pid) => pid,
            Err(e) => {
                let err_msg = format!("Failed to spawn cloud-hypervisor process: {}", e);

                let mut vms = self
                    .vms
                    .lock()
                    .expect("FATAL: VmManager mutex poisoned - this is a critical PID 1 failure");
                if let Some(vm) = vms.get_mut(vm_id) {
                    vm.state = VmState::Failed(err_msg.clone());
                }

                crate::log!("vm", "ERROR: {}", err_msg);
                return Err(err_msg);
            }
        };

        {
            let mut vms = self
                .vms
                .lock()
                .expect("FATAL: VmManager mutex poisoned - this is a critical PID 1 failure");
            if let Some(vm) = vms.get_mut(vm_id) {
                vm.pid = Some(pid);
                vm.state = VmState::Running;
            }
        }

        crate::log!("vm", "VM {} started successfully with PID {}", vm_id, pid);

        Ok(())
    }

    pub fn stop(&self, vm_id: &str, force: bool) -> Result<(), String> {
        let pid;
        let signal = if force { 9 } else { 15 };

        {
            let mut vms = self
                .vms
                .lock()
                .expect("FATAL: VmManager mutex poisoned - this is a critical PID 1 failure");
            let vm = vms.get_mut(vm_id).ok_or("VM not found")?;

            if vm.state != VmState::Running {
                return Err(format!("VM is not running: {}", vm.state.to_string()));
            }

            pid = vm.pid.ok_or("VM has no PID")?;
            vm.state = VmState::Stopping;
        }

        self.process_manager.stop(pid, signal)?;

        {
            let mut vms = self
                .vms
                .lock()
                .expect("FATAL: VmManager mutex poisoned - this is a critical PID 1 failure");
            if let Some(vm) = vms.get_mut(vm_id) {
                vm.state = VmState::Stopped;
                vm.pid = None;
            }
        }

        Ok(())
    }

    pub fn delete(&self, vm_id: &str) -> Result<(), String> {
        let mut vms = self
            .vms
            .lock()
            .expect("FATAL: VmManager mutex poisoned - this is a critical PID 1 failure");
        let vm = vms.get(vm_id).ok_or("VM not found")?;

        if vm.state == VmState::Running || vm.state == VmState::Starting {
            return Err("VM must be stopped before deletion".to_string());
        }

        vms.remove(vm_id);
        Ok(())
    }

    pub fn list(&self) -> Vec<Vm> {
        let vms = self
            .vms
            .lock()
            .expect("FATAL: VmManager mutex poisoned - this is a critical PID 1 failure");
        vms.values().cloned().collect()
    }

    pub fn get(&self, vm_id: &str) -> Option<Vm> {
        let vms = self
            .vms
            .lock()
            .expect("FATAL: VmManager mutex poisoned - this is a critical PID 1 failure");
        vms.get(vm_id).cloned()
    }
}
