pub mod cloud_hypervisor;
pub mod firecracker;

use crate::vm::{DiskConfig, NetConfig};

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize, Default)]
pub enum VmmType {
    #[default]
    CloudHypervisor,
    Firecracker,
    Qemu,
}

impl std::fmt::Display for VmmType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            VmmType::CloudHypervisor => write!(f, "cloud-hypervisor"),
            VmmType::Firecracker => write!(f, "firecracker"),
            VmmType::Qemu => write!(f, "qemu"),
        }
    }
}

impl std::str::FromStr for VmmType {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "cloud-hypervisor" | "ch" => Ok(VmmType::CloudHypervisor),
            "firecracker" | "fc" => Ok(VmmType::Firecracker),
            "qemu" => Ok(VmmType::Qemu),
            _ => Err(format!("Unknown VMM type: {}", s)),
        }
    }
}

#[derive(Debug, Clone)]
pub struct VmmConfig {
    pub vm_id: String,
    pub cpus: i32,
    pub memory_mb: i64,
    pub kernel: Option<String>,
    pub initrd: Option<String>,
    pub cmdline: Option<String>,
    pub disks: Vec<DiskConfig>,
    pub networks: Vec<NetConfig>,
}

#[derive(Debug)]
pub struct VmmStartResult {
    pub pid: i32,
}

pub enum VmmBackend {
    CloudHypervisor(cloud_hypervisor::CloudHypervisorBackend),
    Firecracker(firecracker::FirecrackerBackend),
}

impl VmmBackend {
    pub async fn start(
        &self,
        config: VmmConfig,
        process_manager: &crate::process::ProcessManager,
    ) -> Result<VmmStartResult, String> {
        match self {
            VmmBackend::CloudHypervisor(backend) => backend.start(config, process_manager).await,
            VmmBackend::Firecracker(backend) => backend.start(config, process_manager).await,
        }
    }

    pub async fn stop(&self, pid: i32, force: bool) -> Result<(), String> {
        match self {
            VmmBackend::CloudHypervisor(backend) => backend.stop(pid, force).await,
            VmmBackend::Firecracker(backend) => backend.stop(pid, force).await,
        }
    }
}

pub fn create_backend(vmm_type: VmmType) -> VmmBackend {
    match vmm_type {
        VmmType::CloudHypervisor => {
            VmmBackend::CloudHypervisor(cloud_hypervisor::CloudHypervisorBackend::new())
        }
        VmmType::Firecracker => VmmBackend::Firecracker(firecracker::FirecrackerBackend::new()),
        VmmType::Qemu => {
            panic!("QEMU backend not yet implemented");
        }
    }
}
