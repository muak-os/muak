//! Sans-io transport boundary for TPM2 command exchange.

use crate::error::Result;

/// Carries encoded TPM2 commands to a device and returns the raw response.
pub trait Transport {
    /// Exchanges an encoded command for its raw response.
    ///
    /// # Errors
    ///
    /// Returns an error when the underlying device fails to exchange the command.
    fn transact(&mut self, request: &[u8]) -> Result<&[u8]>;
}
