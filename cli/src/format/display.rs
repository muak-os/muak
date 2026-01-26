use crate::client::{Hypervisor, VmState};

/// Converts a VM state integer to a display string.
pub fn vm_state_to_string(state: i32) -> &'static str {
    match VmState::try_from(state).unwrap_or(VmState::Unspecified) {
        VmState::Unspecified => "unknown",
        VmState::Created => "created",
        VmState::Starting => "starting",
        VmState::Running => "running",
        VmState::Stopping => "stopping",
        VmState::Stopped => "stopped",
        VmState::Failed => "failed",
    }
}

/// Converts a hypervisor integer to a display string.
pub fn hypervisor_to_string(hypervisor: i32) -> &'static str {
    match Hypervisor::try_from(hypervisor).unwrap_or(Hypervisor::Unspecified) {
        Hypervisor::Unspecified => "unknown",
        Hypervisor::Firecracker => "firecracker",
        Hypervisor::CloudHypervisor => "cloud-hypervisor",
        Hypervisor::Qemu => "qemu",
    }
}
