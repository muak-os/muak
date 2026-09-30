//! Correlation between the baked tools version and catalog release lines.

use crate::error::{KataError, Result};

/// This binary's workspace version, in catalog line convention (`v` prefix).
pub const LINE: &str = concat!("v", env!("CARGO_PKG_VERSION"));

/// Refuses a release that would create a foreign-version line.
///
/// # Errors
///
/// Returns an error when `release` names a new line other than [`LINE`].
pub fn ensure_line(release: &str, line_exists: bool, bypass: bool) -> Result<()> {
    if bypass || line_exists || release == LINE {
        return Ok(());
    }

    Err(KataError::Version(format!(
        "refusing to create line '{release}' with the {LINE} tools image; \
         tag and release '{release}' first, or pass --force for a dev scratch line"
    )))
}

#[cfg(test)]
mod tests {
    use super::{LINE, ensure_line};

    #[test]
    fn accepts_the_matching_line() {
        // ARRANGE
        let release = LINE;

        // ACT
        let result = ensure_line(release, false, false);

        // ASSERT
        result.expect("matching line must be accepted");
    }

    #[test]
    fn accepts_existing_and_bypassed_lines() {
        // ARRANGE
        let foreign = "v99.0.0";

        // ACT / ASSERT
        ensure_line(foreign, true, false).expect("existing line must be accepted");
        ensure_line(foreign, false, true).expect("bypassed line must be accepted");
    }

    #[test]
    fn refuses_a_foreign_new_line() {
        // ARRANGE
        let foreign = "v99.0.0";

        // ACT
        let error = ensure_line(foreign, false, false).expect_err("foreign line must be refused");

        // ASSERT
        let message = error.to_string();
        assert!(message.contains(foreign));
        assert!(message.contains(LINE));
    }
}
