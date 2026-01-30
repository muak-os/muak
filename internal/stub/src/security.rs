//! Security-related EFI variable checks.

use uefi::CStr16;
use uefi::runtime::VariableVendor;

/// Reads a single-byte EFI variable from the global vendor namespace.
fn read_bool_variable(name: &str) -> bool {
    let mut name_buf = [0u16; 16];
    let name = match CStr16::from_str_with_buf(name, &mut name_buf) {
        Ok(n) => n,
        Err(_) => return false,
    };

    let mut buf = [0u8; 1];
    match uefi::runtime::get_variable(name, &VariableVendor::GLOBAL_VARIABLE, &mut buf) {
        Ok((data, _)) => data[0] == 1,
        Err(_) => false,
    }
}

/// Returns whether the system is in UEFI Setup Mode.
pub fn is_setup_mode() -> bool {
    read_bool_variable("SetupMode")
}

/// Returns whether Secure Boot is enabled.
pub fn is_secure_boot_enabled() -> bool {
    read_bool_variable("SecureBoot")
}
