//! TPM2 error types.

use thiserror::Error;

/// TPM2 operation error types.
#[expect(
    clippy::module_name_repetitions,
    reason = "The public error type intentionally matches the crate name"
)]
#[derive(Debug, Error)]
pub enum Tpm2Error {
    /// TPM2 device is not available, as reported by a transport implementation.
    #[error("TPM2 device not found")]
    DeviceNotFound,

    /// Underlying I/O operation failed, as reported by a transport implementation.
    #[cfg(feature = "linux")]
    #[error("TPM2 I/O error: {0}")]
    Io(#[from] std::io::Error),

    /// Transport failed to exchange the command, with a code from the transport implementation.
    #[error("TPM2 transport error: code {code}")]
    Transport {
        /// Platform-specific failure code reported by the transport.
        code: i32,
    },

    /// TPM2 command returned a failure response code.
    #[error("TPM2 command failed: RC=0x{0:08X}")]
    TpmError(u32),

    /// TPM2 response is shorter than required.
    #[error("TPM2 response too short: expected {expected}, got {actual}")]
    ResponseTooShort {
        /// Expected response length in bytes.
        expected: usize,
        /// Actual response length in bytes.
        actual: usize,
    },

    /// TPM2 response is larger than the transport buffer.
    #[error("TPM2 response too large: {actual} bytes exceeds {max} bytes")]
    ResponseTooLarge {
        /// Maximum buffer length in bytes.
        max: usize,
        /// Reported response length in bytes.
        actual: usize,
    },

    /// TPM2 response tag is not recognized.
    #[error("TPM2 response tag mismatch")]
    BadResponseTag,

    /// Sealed blob has an invalid format.
    #[error("invalid sealed blob format")]
    InvalidBlob,

    /// Random number generation failed, as reported by a transport implementation.
    #[error("random number generation failed")]
    RngFailed,

    /// Command buffer exceeds the protocol limit.
    #[error("TPM2 buffer too large: {actual} bytes exceeds {max} bytes")]
    BufferTooLarge {
        /// Requested buffer length in bytes.
        actual: usize,
        /// Maximum supported buffer length in bytes.
        max: usize,
    },

    /// Caller-provided output buffer is too small for the produced bytes.
    #[error("TPM2 output buffer too small: needs {needed} bytes, has {provided}")]
    OutputTooSmall {
        /// Required output length in bytes.
        needed: usize,
        /// Provided output buffer length in bytes.
        provided: usize,
    },
}

/// Result type for TPM2 operations.
pub type Result<T> = core::result::Result<T, Tpm2Error>;

#[cfg(test)]
mod tests {
    use alloc::string::ToString as _;

    use super::*;

    #[test]
    fn transport_displays_code() {
        // ARRANGE
        let error = Tpm2Error::Transport { code: -5_i32 };

        // ACT
        let displayed = error.to_string();

        // ASSERT
        assert_eq!(
            displayed, "TPM2 transport error: code -5",
            "transport error should display its code"
        );
    }
}
