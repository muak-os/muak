//! Lazily initialized tokio runtime shared by the synchronous public API.

use std::io;
use std::sync::OnceLock;

use tokio::runtime::Runtime;

use crate::error::{KociError, Result};

/// Lazily initialized multi-thread runtime driving the async machinery.
static RUNTIME: OnceLock<core::result::Result<Runtime, io::Error>> = OnceLock::new();

/// Return the lazily initialized runtime, propagating startup failures.
pub(crate) fn runtime() -> Result<&'static Runtime> {
    RUNTIME
        .get_or_init(Runtime::new)
        .as_ref()
        .map_err(|error| KociError::NetworkError(format!("failed to start async runtime: {error}")))
}
