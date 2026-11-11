pub(crate) mod cloud_hypervisor;
pub(crate) mod firecracker;
pub(crate) mod qemu;

use std::path::PathBuf;

use anyhow::Result;
use cloud_hypervisor::Driver as CloudHypervisorDriver;
use firecracker::Driver as FirecrackerDriver;
use qemu::Driver as QemuDriver;

use crate::proto::vm::Hypervisor as HypervisorType;

#[derive(Debug, Clone)]
pub(crate) struct VmStartConfig {
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
pub(crate) struct DiskConfig {
    pub path: PathBuf,
    pub readonly: bool,
}

#[derive(Debug)]
pub(crate) struct VmProcess {
    pub pid: u32,
}

pub(crate) enum HypervisorImpl {
    Firecracker(FirecrackerDriver),
    CloudHypervisor(CloudHypervisorDriver),
    Qemu(QemuDriver),
}

impl HypervisorImpl {
    pub(crate) async fn start(&self, config: &VmStartConfig) -> Result<VmProcess> {
        match *self {
            HypervisorImpl::Firecracker(ref hypervisor) => hypervisor.start(config).await,
            HypervisorImpl::CloudHypervisor(ref hypervisor) => hypervisor.start(config),
            HypervisorImpl::Qemu(ref hypervisor) => hypervisor.start(config),
        }
    }

    pub(crate) fn stop(&self, pid: u32, force: bool) -> Result<()> {
        match *self {
            HypervisorImpl::Firecracker(_) => FirecrackerDriver::stop(pid, force),
            HypervisorImpl::CloudHypervisor(_) => CloudHypervisorDriver::stop(pid, force),
            HypervisorImpl::Qemu(_) => QemuDriver::stop(pid, force),
        }
    }
}

pub(crate) fn create_hypervisor(hypervisor_type: HypervisorType) -> HypervisorImpl {
    match hypervisor_type {
        HypervisorType::Firecracker => HypervisorImpl::Firecracker(FirecrackerDriver::new()),
        HypervisorType::CloudHypervisor => {
            HypervisorImpl::CloudHypervisor(CloudHypervisorDriver::new())
        }
        HypervisorType::Qemu | HypervisorType::Unspecified => {
            HypervisorImpl::Qemu(QemuDriver::new())
        }
    }
}
