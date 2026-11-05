//! Preflight check outcome and the refusal aggregation used by the installer and wipe paths.

use anyhow::{Result, bail};

/// Outcome of one install preflight check.
pub(crate) struct Check {
    pub name: &'static str,
    pub passed: bool,
    pub detail: String,
}

impl Check {
    pub(crate) fn pass(name: &'static str, detail: String) -> Self {
        Self {
            name,
            passed: true,
            detail,
        }
    }

    pub(crate) fn fail(name: &'static str, detail: String) -> Self {
        Self {
            name,
            passed: false,
            detail,
        }
    }
}

/// Returns an error listing every refusing check, if any.
pub(crate) fn enforce(checks: &[Check]) -> Result<()> {
    let refused: Vec<&str> = checks
        .iter()
        .filter(|check| !check.passed)
        .map(|check| check.detail.as_str())
        .collect();

    if refused.is_empty() {
        return Ok(());
    }

    bail!("{}", refused.join("; "))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn passing(name: &'static str) -> Check {
        Check::pass(name, "ok".to_owned())
    }

    #[test]
    fn enforce_accepts_passing_checks() {
        // ARRANGE
        let checks = vec![passing("device_exists"), passing("disk_size")];

        // ACT
        let result = enforce(&checks);

        // ASSERT
        result.expect("passing checks must be accepted");
    }

    #[test]
    fn enforce_reports_every_refusal() {
        // ARRANGE
        let checks = vec![
            passing("device_exists"),
            Check::fail("disk_size", "too small".to_owned()),
            Check::fail("empty_disk", "not empty".to_owned()),
        ];

        // ACT
        let error = enforce(&checks).expect_err("refusals must fail");

        // ASSERT
        let message = error.to_string();
        assert_eq!(message, "too small; not empty");
    }
}
