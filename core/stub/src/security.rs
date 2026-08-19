//! Security-related EFI variable checks.

use uefi::CStr16;
use uefi::runtime::{VariableVendor, get_variable};

/// Returns whether the system is in UEFI Setup Mode.
#[must_use]
pub fn is_setup_mode() -> bool {
    read_bool_variable("SetupMode")
}

/// Returns whether Secure Boot is enabled.
#[must_use]
pub fn is_secure_boot_enabled() -> bool {
    read_bool_variable("SecureBoot")
}

/// Reads a single-byte EFI variable from the global vendor namespace.
fn read_bool_variable(name: &str) -> bool {
    let mut name_buf = [0_u16; 16];
    let Ok(name) = CStr16::from_str_with_buf(name, &mut name_buf) else {
        return false;
    };

    let mut buf = [0_u8; 1];
    match get_variable(name, &VariableVendor::GLOBAL_VARIABLE, &mut buf) {
        Ok((data, _)) => data.first().copied() == Some(1),
        Err(_) => false,
    }
}
