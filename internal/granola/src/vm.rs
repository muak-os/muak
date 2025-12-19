use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fmt;
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
    pub kernel: Option<String>,
    pub initrd: Option<String>,
    pub cmdline: Option<String>,
    pub disks: Vec<DiskConfig>,
    pub networks: Vec<NetConfig>,
    pub vmm_type: crate::vmm::VmmType,
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

impl fmt::Display for VmState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            VmState::Created => write!(f, "created"),
            VmState::Starting => write!(f, "starting"),
            VmState::Running => write!(f, "running"),
            VmState::Stopping => write!(f, "stopping"),
            VmState::Stopped => write!(f, "stopped"),
            VmState::Failed(e) => write!(f, "failed: {}", e),
        }
    }
}

#[derive(Clone)]
pub struct VmManager {
    vms: Arc<Mutex<HashMap<String, Vm>>>,
    process_manager: crate::process::ProcessManager,
    network: Arc<crate::network::NetworkActorHandle>,
}

impl VmManager {
    pub fn new(
        process_manager: crate::process::ProcessManager,
        network: Arc<crate::network::NetworkActorHandle>,
    ) -> Self {
        Self {
            vms: Arc::new(Mutex::new(HashMap::new())),
            process_manager,
            network,
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

    pub async fn start(&self, vm_id: &str) -> Result<(), String> {
        let vm_config;
        let vm_name;
        let vmm_type;

        {
            let mut vms = self
                .vms
                .lock()
                .expect("FATAL: VmManager mutex poisoned - this is a critical PID 1 failure");
            let vm = vms.get_mut(vm_id).ok_or("VM not found")?;

            if vm.state != VmState::Created && vm.state != VmState::Stopped {
                let err_msg = format!("Cannot start VM in state: {}", vm.state);
                kmsg::error!(@ "vm", "{}", err_msg);
                return Err(err_msg);
            }

            vm.state = VmState::Starting;
            vm_name = vm.name.clone();
            vm_config = vm.config.clone();
            vmm_type = vm.config.vmm_type.clone();
        }

        kmsg::info!(
            @ "vm",
            "Starting VM {} ({}) using {}",
            vm_id,
            vm_name,
            vmm_type
        );

        let mut network_configs = vm_config.networks.clone();
        if network_configs.is_empty() {
            kmsg::info!(@ "vm", "Auto-creating network configuration for VM {}", vm_id);

            let tap_name = format!("tap-{}", &vm_id[3..8]);
            let mac_bytes = crate::network::generate_mac_address(vm_id);
            let mac_addr = crate::network::format_mac_address(&mac_bytes);

            kmsg::info!(
                @ "vm",
                "Creating TAP device {} with MAC {}",
                tap_name,
                mac_addr
            );

            match self.network.add_tap(tap_name.clone()).await {
                Ok(_iface) => {
                    kmsg::info!(@ "vm", "TAP device {} configured and attached", tap_name);
                }
                Err(e) => {
                    let err_msg = format!("Failed to setup TAP {}: {}", tap_name, e);
                    let mut vms = self.vms.lock().expect("FATAL: VmManager mutex poisoned");
                    if let Some(vm) = vms.get_mut(vm_id) {
                        vm.state = VmState::Failed(err_msg.clone());
                    }
                    kmsg::error!(@ "vm", "{}", err_msg);
                    return Err(err_msg);
                }
            }

            network_configs.push(NetConfig {
                tap: tap_name.clone(),
                mac: mac_addr,
            });
            let mut vms = self.vms.lock().expect("FATAL: VmManager mutex poisoned");
            if let Some(vm) = vms.get_mut(vm_id) {
                vm.config.networks = network_configs.clone();
            }
        }

        let backend = crate::vmm::create_backend(vmm_type);
        let vmm_config = crate::vmm::VmmConfig {
            vm_id: vm_id.to_string(),
            cpus: vm_config.cpus,
            memory_mb: vm_config.memory_mb,
            kernel: vm_config.kernel.clone(),
            initrd: vm_config.initrd.clone(),
            cmdline: vm_config.cmdline.clone(),
            disks: vm_config.disks.clone(),
            networks: network_configs.clone(),
        };

        let result = backend.start(vmm_config, &self.process_manager).await;
        let pid = match result {
            Ok(start_result) => start_result.pid,
            Err(e) => {
                let err_msg = format!("Failed to start VM: {}", e);
                let mut vms = self
                    .vms
                    .lock()
                    .expect("FATAL: VmManager mutex poisoned - this is a critical PID 1 failure");
                if let Some(vm) = vms.get_mut(vm_id) {
                    vm.state = VmState::Failed(err_msg.clone());
                }
                kmsg::error!(@ "vm", "{}", err_msg);
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

        kmsg::info!(@ "vm", "VM {} started successfully with PID {}", vm_id, pid);
        Ok(())
    }

    pub async fn stop(&self, vm_id: &str, force: bool) -> Result<(), String> {
        let pid;
        let signal = if force { 9 } else { 15 };
        let tap_devices;

        {
            let mut vms = self
                .vms
                .lock()
                .expect("FATAL: VmManager mutex poisoned - this is a critical PID 1 failure");
            let vm = vms.get_mut(vm_id).ok_or("VM not found")?;

            if vm.state != VmState::Running {
                return Err(format!("VM is not running: {}", vm.state));
            }

            pid = vm.pid.ok_or("VM has no PID")?;
            vm.state = VmState::Stopping;
            tap_devices = vm
                .config
                .networks
                .iter()
                .map(|n| n.tap.clone())
                .collect::<Vec<_>>();
        }

        self.process_manager.stop(pid, signal)?;

        for tap_name in &tap_devices {
            kmsg::info!(@ "vm", "Cleaning up TAP device: {}", tap_name);
            if let Err(e) = self.network.delete_tap(tap_name.clone()).await {
                kmsg::warn!(
                    @ "vm",
                    "Failed to delete TAP device {}: {}",
                    tap_name,
                    e
                );
            }
        }

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
