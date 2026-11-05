//! Conversion of preflight checks into their protobuf representation.

use super::proto::provision::PreflightCheck;
use crate::check::Check;

/// Converts a preflight check into its protobuf representation.
pub(crate) fn to_proto(check: Check) -> PreflightCheck {
    PreflightCheck {
        name: check.name.to_owned(),
        passed: check.passed,
        detail: check.detail,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preflight_checks_keep_their_outcome() {
        // ARRANGE
        let check = Check::fail("disk_size", "too small".to_owned());

        // ACT
        let proto = to_proto(check);

        // ASSERT
        assert_eq!(proto.name, "disk_size");
        assert!(!proto.passed);
        assert_eq!(proto.detail, "too small");
    }
}
