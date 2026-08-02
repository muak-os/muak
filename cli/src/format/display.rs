use crate::client::vm_service::{Hypervisor, VmState};

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vm_state_unspecified() {
        assert_eq!(vm_state_to_string(0), "unknown");
    }

    #[test]
    fn vm_state_created() {
        assert_eq!(vm_state_to_string(1), "created");
    }

    #[test]
    fn vm_state_starting() {
        assert_eq!(vm_state_to_string(2), "starting");
    }

    #[test]
    fn vm_state_running() {
        assert_eq!(vm_state_to_string(3), "running");
    }

    #[test]
    fn vm_state_stopping() {
        assert_eq!(vm_state_to_string(4), "stopping");
    }

    #[test]
    fn vm_state_stopped() {
        assert_eq!(vm_state_to_string(5), "stopped");
    }

    #[test]
    fn vm_state_failed() {
        assert_eq!(vm_state_to_string(6), "failed");
    }

    #[test]
    fn vm_state_invalid_falls_back_to_unknown() {
        // ARRANGE
        let invalid_state_1 = 99;
        let invalid_state_2 = -1;

        // ACT & ASSERT
        assert_eq!(vm_state_to_string(invalid_state_1), "unknown");
        assert_eq!(vm_state_to_string(invalid_state_2), "unknown");
    }

    #[test]
    fn hypervisor_unspecified() {
        assert_eq!(hypervisor_to_string(0), "unknown");
    }

    #[test]
    fn hypervisor_firecracker() {
        assert_eq!(hypervisor_to_string(1), "firecracker");
    }

    #[test]
    fn hypervisor_cloud_hypervisor() {
        assert_eq!(hypervisor_to_string(2), "cloud-hypervisor");
    }

    #[test]
    fn hypervisor_qemu() {
        assert_eq!(hypervisor_to_string(3), "qemu");
    }

    #[test]
    fn hypervisor_invalid_falls_back_to_unknown() {
        // ARRANGE
        let invalid_hypervisor_1 = 99;
        let invalid_hypervisor_2 = -1;

        // ACT & ASSERT
        assert_eq!(hypervisor_to_string(invalid_hypervisor_1), "unknown");
        assert_eq!(hypervisor_to_string(invalid_hypervisor_2), "unknown");
    }
}
