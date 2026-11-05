//! Secure Boot key enrollment check shared by the dry-run plan and the installer.

use crate::check::Check;

/// Refuses Secure Boot key enrollment when the firmware is not in Setup Mode.
pub(crate) fn check(secureboot: bool) -> Check {
    let name = "secureboot_setup_mode";
    if !secureboot {
        return Check::pass(
            name,
            "Secure Boot key enrollment is not requested".to_owned(),
        );
    }

    if firmware::secureboot::system().is_ok_and(|state| state.setup_mode) {
        return Check::pass(name, "Firmware is in Setup Mode".to_owned());
    }

    Check::fail(
        name,
        "Firmware is not in Setup Mode, cannot enroll Secure Boot keys. Reset your firmware to \
         Setup Mode and try again or disable the secureboot option in the config."
            .to_owned(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn check_passes_when_enrollment_is_not_requested() {
        // ARRANGE
        let secureboot = false;

        // ACT
        let result = check(secureboot);

        // ASSERT
        assert!(result.passed, "{}", result.detail);
        assert_eq!(result.name, "secureboot_setup_mode");
    }
}
