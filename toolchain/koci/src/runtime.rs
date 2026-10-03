//! Lazily initialized tokio runtime shared by the synchronous public API.

use std::sync::OnceLock;

use tokio::runtime::Runtime;

use crate::error::{KociError, Result};

/// Lazily initialized multi-thread runtime driving the async machinery.
static RUNTIME: OnceLock<core::result::Result<Runtime, String>> = OnceLock::new();

/// Return the lazily initialized runtime, propagating startup failures.
pub(crate) fn runtime() -> Result<&'static Runtime> {
    RUNTIME
        .get_or_init(|| {
            Runtime::new().map_err(|error| format!("failed to start async runtime: {error}"))
        })
        .as_ref()
        .map_err(|message| KociError::IoError(std::io::Error::other(message.clone())))
}
