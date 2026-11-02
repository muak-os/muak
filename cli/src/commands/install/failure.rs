//! Install failures and the process exit status they report.

/// Install failure carrying the process exit status it must report.
#[derive(Debug)]
pub struct Failure {
    code: i32,
    message: String,
}

impl Failure {
    /// Creates a usage or configuration failure, reported as exit code 2.
    pub(crate) fn usage(message: impl Into<String>) -> Self {
        Self {
            code: 2,
            message: message.into(),
        }
    }

    /// Creates a preflight refusal, reported as exit code 3.
    pub(crate) fn preflight(message: impl Into<String>) -> Self {
        Self {
            code: 3,
            message: message.into(),
        }
    }

    /// Creates a declined confirmation, reported as exit code 4.
    pub(crate) fn declined(message: impl Into<String>) -> Self {
        Self {
            code: 4,
            message: message.into(),
        }
    }

    /// Creates a failure after writes started, reported as exit code 5.
    pub(crate) fn incomplete(message: impl Into<String>) -> Self {
        Self {
            code: 5,
            message: message.into(),
        }
    }

    /// Returns the process exit status of this failure.
    pub(crate) fn code(&self) -> i32 {
        self.code
    }
}

impl std::fmt::Display for Failure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for Failure {}
