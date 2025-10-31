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
    network_manager: Arc<crate::network::NetworkManager>,
}

impl VmManager {
    pub fn new(
        process_manager: crate::process::ProcessManager,
        network_manager: Arc<crate::network::NetworkManager>,
    ) -> Self {
        Self {
            vms: Arc::new(Mutex::new(HashMap::new())),
            process_manager,
            network_manager,
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
                crate::log!("vm", "{}", err_msg);
                return Err(err_msg);
            }

            vm.state = VmState::Starting;
            vm_name = vm.name.clone();
            vm_config = vm.config.clone();
            vmm_type = vm.config.vmm_type.clone();
        }

        crate::log!(
            "vm",
            "Starting VM {} ({}) using {}",
            vm_id,
            vm_name,
            vmm_type
        );

        let mut network_configs = vm_config.networks.clone();
        if network_configs.is_empty() {
            crate::log!("vm", "Auto-creating network configuration for VM {}", vm_id);

            // Generate TAP device name from VM ID (tap-xxxxx where xxxxx are first 5 chars of UUID)
            let tap_name = format!("tap-{}", &vm_id[3..8]);

            // Generate deterministic MAC address from VM ID
            let mac_bytes = crate::network::generate_mac_address(vm_id);
            let mac_addr = crate::network::format_mac_address(&mac_bytes);

            crate::log!(
                "vm",
                "Creating TAP device {} with MAC {}",
                tap_name,
                mac_addr
            );

            let handle = self.network_manager.get_handle();
            if let Err(e) = crate::network::create_tap(&tap_name).await {
                let err_msg = format!("Failed to create TAP device: {}", e);
                let mut vms = self.vms.lock().expect("FATAL: VmManager mutex poisoned");
                if let Some(vm) = vms.get_mut(vm_id) {
                    vm.state = VmState::Failed(err_msg.clone());
                }
                crate::log!("vm", "ERROR: {}", err_msg);
                return Err(err_msg);
            }

            let err_msg = {
                match crate::network::bring_up_tap(&handle, &tap_name).await {
                    Ok(_) => None,
                    Err(e) => Some(format!("Failed to bring up TAP device: {}", e)),
                }
            };
            if let Some(err_msg) = err_msg {
                {
                    let mut vms = self.vms.lock().expect("FATAL: VmManager mutex poisoned");
                    if let Some(vm) = vms.get_mut(vm_id) {
                        vm.state = VmState::Failed(err_msg.clone());
                    }
                }
                crate::log!("vm", "ERROR: {}", err_msg);
                let _ = crate::network::delete_tap(&handle, &tap_name).await;
                return Err(err_msg);
            }

            crate::log!(
                "vm",
                "Attaching TAP {} to bridge for VM {}",
                tap_name,
                vm_id
            );
            let err_msg = {
                let bridge_name = crate::network::LAN_BRIDGE_NAME;

                match crate::network::attach_to_bridge(&handle, &tap_name, bridge_name).await {
                    Ok(_) => None,
                    Err(e) => Some(format!("Failed to attach TAP to bridge: {}", e)),
                }
            };
            if let Some(err_msg) = err_msg {
                {
                    let mut vms = self.vms.lock().expect("FATAL: VmManager mutex poisoned");
                    if let Some(vm) = vms.get_mut(vm_id) {
                        vm.state = VmState::Failed(err_msg.clone());
                    }
                }
                crate::log!("vm", "ERROR: {}", err_msg);
                let _ = crate::network::delete_tap(&handle, &tap_name).await;
                return Err(err_msg);
            }

            crate::log!("vm", "TAP device {} configured successfully", tap_name);

            network_configs.push(NetConfig {
                tap: tap_name.clone(),
                mac: mac_addr,
            });

            let mut vms = self.vms.lock().expect("FATAL: VmManager mutex poisoned");
            if let Some(vm) = vms.get_mut(vm_id) {
                vm.config.networks = network_configs.clone();
            }
        }

        // Create VMM backend
        let backend = crate::vmm::create_backend(vmm_type);

        // Prepare VMM configuration
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

        // Start the VM using the backend
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

        let handle = self.network_manager.get_handle();
        for tap_name in &tap_devices {
            crate::log!("vm", "Cleaning up TAP device: {}", tap_name);
            if let Err(e) = crate::network::delete_tap(&handle, tap_name).await {
                crate::log!(
                    "vm",
                    "WARNING: Failed to delete TAP device {}: {}",
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
