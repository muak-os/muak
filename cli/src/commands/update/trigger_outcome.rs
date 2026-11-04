use tonic::{Code, Status};

/// What the CLI does after the update trigger RPC returned an error status.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum TriggerOutcome {
    /// The connection ended the way it does when the machine reboots: poll status.
    AssumeReboot,
    /// The server rejected the trigger: report it and stop.
    Failed,
}

/// Classifies the gRPC code of a failed update trigger RPC.
pub(crate) fn trigger_outcome(code: Code) -> TriggerOutcome {
    match code {
        Code::Unavailable
        | Code::Cancelled
        | Code::DeadlineExceeded
        | Code::Unknown
        | Code::Aborted
        | Code::Internal => TriggerOutcome::AssumeReboot,
        Code::PermissionDenied
        | Code::Unauthenticated
        | Code::NotFound
        | Code::InvalidArgument
        | Code::FailedPrecondition
        | Code::AlreadyExists
        | Code::ResourceExhausted
        | Code::Unimplemented
        | Code::OutOfRange
        | Code::DataLoss
        | Code::Ok => TriggerOutcome::Failed,
    }
}

/// Progress line shown when the trigger connection ended and the machine may be rebooting.
pub(crate) fn trigger_reboot_notice(status: &Status) -> String {
    format!(
        "Update trigger connection ended ({}); the machine may be rebooting",
        status_detail(status)
    )
}

/// Error message for a trigger the server definitely rejected.
pub(crate) fn trigger_failure_message(status: &Status) -> String {
    format!("Update trigger failed ({})", status_detail(status))
}

// Debug prints the gRPC code name; Display prints a human sentence instead.
fn status_detail(status: &Status) -> String {
    if status.message().is_empty() {
        format!("{:?}", status.code())
    } else {
        format!("{:?}: {}", status.code(), status.message())
    }
}

#[cfg(test)]
mod tests {
    use tonic::{Code, Status};

    use super::{TriggerOutcome, trigger_failure_message, trigger_outcome, trigger_reboot_notice};

    const REBOOT_CODES: [Code; 6] = [
        Code::Unavailable,
        Code::Cancelled,
        Code::DeadlineExceeded,
        Code::Unknown,
        Code::Aborted,
        Code::Internal,
    ];

    const FAILED_CODES: [Code; 11] = [
        Code::PermissionDenied,
        Code::Unauthenticated,
        Code::NotFound,
        Code::InvalidArgument,
        Code::FailedPrecondition,
        Code::AlreadyExists,
        Code::ResourceExhausted,
        Code::Unimplemented,
        Code::OutOfRange,
        Code::DataLoss,
        Code::Ok,
    ];

    #[test]
    fn reboot_codes_continue_to_the_status_poll() {
        // ARRANGE
        // REBOOT_CODES holds the transport codes a rebooting machine produces.

        // ACT
        let outcomes: Vec<TriggerOutcome> =
            REBOOT_CODES.iter().copied().map(trigger_outcome).collect();

        // ASSERT
        assert_eq!(
            outcomes,
            vec![TriggerOutcome::AssumeReboot; REBOOT_CODES.len()]
        );
    }

    #[test]
    fn rejection_codes_fail_immediately() {
        // ARRANGE
        // FAILED_CODES holds the codes the server returns for a refused trigger.

        // ACT
        let outcomes: Vec<TriggerOutcome> =
            FAILED_CODES.iter().copied().map(trigger_outcome).collect();

        // ASSERT
        assert_eq!(outcomes, vec![TriggerOutcome::Failed; FAILED_CODES.len()]);
    }

    #[test]
    fn unavailability_is_treated_as_a_reboot() {
        // ARRANGE
        let code = Code::Unavailable;

        // ACT
        let outcome = trigger_outcome(code);

        // ASSERT
        assert_eq!(outcome, TriggerOutcome::AssumeReboot);
    }

    #[test]
    fn permission_denied_is_a_definite_failure() {
        // ARRANGE
        let code = Code::PermissionDenied;

        // ACT
        let outcome = trigger_outcome(code);

        // ASSERT
        assert_eq!(outcome, TriggerOutcome::Failed);
    }

    #[test]
    fn failure_message_carries_the_server_message() {
        // ARRANGE
        let status = Status::permission_denied("caller is not allowed to trigger updates");

        // ACT
        let message = trigger_failure_message(&status);

        // ASSERT
        assert!(
            message.contains("caller is not allowed to trigger updates"),
            "server message missing from '{message}'"
        );
        assert!(
            message.contains("PermissionDenied"),
            "code missing from '{message}'"
        );
    }

    #[test]
    fn failure_message_survives_an_empty_server_message() {
        // ARRANGE
        let status = Status::new(Code::NotFound, "");

        // ACT
        let message = trigger_failure_message(&status);

        // ASSERT
        assert_eq!(message, "Update trigger failed (NotFound)");
    }

    #[test]
    fn reboot_notice_says_the_machine_may_be_rebooting() {
        // ARRANGE
        let status = Status::unavailable("connection reset by peer");

        // ACT
        let notice = trigger_reboot_notice(&status);

        // ASSERT
        assert!(notice.contains("rebooting"), "no reboot hint in '{notice}'");
        assert!(
            notice.contains("connection reset by peer"),
            "server message missing from '{notice}'"
        );
    }
}
