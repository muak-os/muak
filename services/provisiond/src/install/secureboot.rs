//! Secure Boot enforcement and key enrollment checks shared by the plan and installer.

use firmware::nvram::VariableStore as _;
use firmware::secureboot::{Mismatch, State};

use crate::check::Check;

const ENFORCEMENT: &str = "secureboot_enforcement";
const SETUP_MODE: &str = "secureboot_setup_mode";
const FIRMWARE: &str = "secureboot_firmware";

/// Checks the firmware posture against the configured Secure Boot intent.
pub(crate) fn system_checks(secureboot: bool) -> Vec<Check> {
    let store = firmware::system();
    if !store.is_boot() || !store.is_available() {
        return vec![unreadable()];
    }

    let state = firmware::secureboot::state(&store).ok();

    evaluate(secureboot, state.as_ref())
}

fn evaluate(secureboot: bool, state: Option<&State>) -> Vec<Check> {
    let Some(state) = state else {
        return vec![unreadable()];
    };

    vec![
        enforcement(secureboot, *state),
        setup_mode(secureboot, *state),
    ]
}

fn enforcement(secureboot: bool, state: State) -> Check {
    if secureboot {
        return Check::pass(
            ENFORCEMENT,
            "Secure Boot key enrollment implies enforcement".to_owned(),
        );
    }

    // The mismatch arm for a disabled config only inspects enforcement, so key
    // ownership is not consulted here.
    if !matches!(
        firmware::secureboot::mismatch(false, &state, false),
        Some(Mismatch::EnforcingWhileDisabled)
    ) {
        return Check::pass(
            ENFORCEMENT,
            "Firmware does not enforce Secure Boot".to_owned(),
        );
    }

    Check::fail(
        ENFORCEMENT,
        "Firmware enforces Secure Boot but the config disables it, so the boot loader would be \
         installed unsigned and fail signature verification at boot. Either enable the \
         `secureboot` option in the config to enroll keys, or disable Secure Boot enforcement in \
         firmware setup."
            .to_owned(),
    )
}

fn setup_mode(secureboot: bool, state: State) -> Check {
    if !secureboot {
        return Check::pass(
            SETUP_MODE,
            "Secure Boot key enrollment is not requested".to_owned(),
        );
    }

    if state.setup_mode {
        return Check::pass(SETUP_MODE, "Firmware is in Setup Mode".to_owned());
    }

    Check::fail(
        SETUP_MODE,
        "Firmware is not in Setup Mode, cannot enroll Secure Boot keys. Reset your firmware to \
         Setup Mode and try again or disable the secureboot option in the config."
            .to_owned(),
    )
}

fn unreadable() -> Check {
    Check::fail(
        FIRMWARE,
        "Secure Boot state could not be read from firmware, so the boot path cannot be verified. \
         Refusing to install a boot loader that may fail signature verification."
            .to_owned(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn firmware_state(enabled: bool, setup_mode: bool) -> State {
        State {
            enabled,
            setup_mode,
            pk_present: !setup_mode,
        }
    }

    #[test]
    fn evaluate_passes_when_enrollment_is_requested_in_setup_mode() {
        // ARRANGE
        let state = firmware_state(false, true);

        // ACT
        let checks = evaluate(true, Some(&state));

        // ASSERT
        assert_eq!(checks.len(), 2, "both checks must be reported");
        assert_eq!(checks[0].name, ENFORCEMENT, "enforcement comes first");
        assert_eq!(checks[1].name, SETUP_MODE, "setup mode comes second");
        assert!(checks[0].passed, "enforcement must pass");
        assert!(checks[1].passed, "setup mode must pass");
    }

    #[test]
    fn evaluate_passes_enforcement_when_firmware_enforces_and_config_enrolls() {
        // ARRANGE
        let state = firmware_state(true, false);

        // ACT
        let checks = evaluate(true, Some(&state));

        // ASSERT
        assert_eq!(checks[0].name, ENFORCEMENT, "enforcement comes first");
        assert!(
            checks[0].passed,
            "enforcing firmware must pass when enrollment is requested"
        );
        assert!(
            !checks[1].passed,
            "user mode must refuse enrollment even when firmware enforces"
        );
    }

    #[test]
    fn evaluate_fails_setup_mode_when_enrollment_is_requested_in_user_mode() {
        // ARRANGE
        let state = firmware_state(false, false);

        // ACT
        let checks = evaluate(true, Some(&state));

        // ASSERT
        assert_eq!(checks.len(), 2, "both checks must be reported");
        assert_eq!(checks[1].name, SETUP_MODE, "setup mode comes second");
        assert!(!checks[1].passed, "user mode must refuse enrollment");
        assert!(
            checks[1].detail.contains("not in Setup Mode"),
            "unexpected detail: {}",
            checks[1].detail
        );
    }

    #[test]
    fn evaluate_fails_enforcement_when_firmware_enforces_and_config_disables() {
        // ARRANGE
        let state = firmware_state(true, false);

        // ACT
        let checks = evaluate(false, Some(&state));

        // ASSERT
        assert_eq!(checks.len(), 2, "both checks must be reported");
        assert_eq!(checks[0].name, ENFORCEMENT, "enforcement comes first");
        assert!(!checks[0].passed, "enforcing firmware must be refused");
        assert!(
            checks[0].detail.contains("unsigned"),
            "detail must name the unsigned boot loader: {}",
            checks[0].detail
        );
        assert!(
            checks[0].detail.contains("signature verification"),
            "detail must name the boot failure: {}",
            checks[0].detail
        );
    }

    #[test]
    fn evaluate_passes_when_enforcement_is_off_and_enrollment_is_not_requested() {
        // ARRANGE
        let state = firmware_state(false, false);

        // ACT
        let checks = evaluate(false, Some(&state));

        // ASSERT
        assert_eq!(checks.len(), 2, "both checks must be reported");
        assert_eq!(checks[0].name, ENFORCEMENT, "enforcement comes first");
        assert_eq!(checks[1].name, SETUP_MODE, "setup mode comes second");
        assert!(checks[0].passed, "enforcement must pass");
        assert!(
            checks[1].passed,
            "setup mode must pass when enrollment is not requested"
        );
    }

    #[test]
    fn evaluate_fails_closed_when_firmware_state_is_unreadable() {
        // ARRANGE
        let state = None;

        // ACT
        let checks = evaluate(false, state);

        // ASSERT
        assert_eq!(checks.len(), 1, "an unreadable state must report one check");
        assert!(!checks[0].passed, "an unreadable state must be refused");
        assert_eq!(checks[0].name, FIRMWARE);
        assert!(
            checks[0].detail.contains("could not be read"),
            "unexpected detail: {}",
            checks[0].detail
        );
    }
}
