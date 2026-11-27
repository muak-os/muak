//! Error type and shared result alias.

use std::io;

use thiserror::Error;

/// Errors reported by the runtime ports.
#[expect(
    clippy::module_name_repetitions,
    reason = "The public error type name intentionally ends with the module name"
)]
#[derive(Debug, Error)]
pub enum RuntimeError {
    /// A port operation failed at the operating system boundary.
    #[error("runtime port operation failed: {0}")]
    Io(#[from] io::Error),
}

impl RuntimeError {
    /// Returns true when the failure is an underlying would-block condition.
    #[must_use = "callers must check whether the failure is retryable"]
    pub fn is_would_block(&self) -> bool {
        matches!(self, Self::Io(error) if error.kind() == io::ErrorKind::WouldBlock)
    }
}

impl From<RuntimeError> for io::Error {
    fn from(error: RuntimeError) -> Self {
        match error {
            RuntimeError::Io(source) => source,
        }
    }
}

/// Shared result type for runtime port operations.
pub type Result<T> = core::result::Result<T, RuntimeError>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn is_would_block_recognizes_the_would_block_kind() {
        // ARRANGE
        let would_block = RuntimeError::Io(io::ErrorKind::WouldBlock.into());
        let timed_out = RuntimeError::Io(io::ErrorKind::TimedOut.into());

        // ACT
        let retryable = would_block.is_would_block();
        let fatal = timed_out.is_would_block();

        // ASSERT
        assert!(retryable);
        assert!(!fatal);
    }

    #[test]
    fn io_conversion_preserves_kind_and_message() {
        // ARRANGE
        let error = RuntimeError::Io(io::Error::new(io::ErrorKind::BrokenPipe, "peer closed"));

        // ACT
        let error = io::Error::from(error);

        // ASSERT
        assert_eq!(error.kind(), io::ErrorKind::BrokenPipe);
        assert_eq!(error.to_string(), "peer closed");
    }
}
