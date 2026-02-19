mod cloud_hypervisor;
mod firecracker;
mod qemu;

use std::path::PathBuf;

use anyhow::Result;
pub use cloud_hypervisor::CloudHypervisorHypervisor;
pub use firecracker::FirecrackerHypervisor;
pub use qemu::QemuHypervisor;

use crate::proto::vm::Hypervisor as HypervisorType;

#[derive(Debug, Clone)]
pub struct VmStartConfig {
    pub vm_id: String,
    pub cpus: u32,
    pub memory_mb: u64,
    pub kernel: PathBuf,
    pub initrd: Option<PathBuf>,
    pub cmdline: String,
    pub disks: Vec<DiskConfig>,
    pub tap_device: String,
    pub mac_address: String,
    pub serial_log_path: PathBuf,
    pub persistent_disk: Option<PathBuf>,
}

#[derive(Debug, Clone)]
pub struct DiskConfig {
    pub path: PathBuf,
    pub readonly: bool,
}

#[derive(Debug)]
pub struct VmProcess {
    pub pid: u32,
}

pub enum HypervisorImpl {
    Firecracker(FirecrackerHypervisor),
    CloudHypervisor(CloudHypervisorHypervisor),
    Qemu(QemuHypervisor),
}

impl HypervisorImpl {
    pub async fn start(&self, config: &VmStartConfig) -> Result<VmProcess> {
        match self {
            HypervisorImpl::Firecracker(h) => h.start(config).await,
            HypervisorImpl::CloudHypervisor(h) => h.start(config).await,
            HypervisorImpl::Qemu(h) => h.start(config).await,
        }
    }

    pub async fn stop(&self, pid: u32, force: bool) -> Result<()> {
        match self {
            HypervisorImpl::Firecracker(h) => h.stop(pid, force).await,
            HypervisorImpl::CloudHypervisor(h) => h.stop(pid, force).await,
            HypervisorImpl::Qemu(h) => h.stop(pid, force).await,
        }
    }
}

pub fn create_hypervisor(hypervisor_type: HypervisorType) -> HypervisorImpl {
    match hypervisor_type {
        HypervisorType::Firecracker => HypervisorImpl::Firecracker(FirecrackerHypervisor::new()),
        HypervisorType::CloudHypervisor => {
            HypervisorImpl::CloudHypervisor(CloudHypervisorHypervisor::new())
        }
        HypervisorType::Qemu | HypervisorType::Unspecified => {
            HypervisorImpl::Qemu(QemuHypervisor::new())
        }
    }
}
