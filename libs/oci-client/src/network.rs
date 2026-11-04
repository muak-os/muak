//! Error construction for registry transport failures.

use core::time::Duration;

use crate::error::ClientError;

pub(crate) fn network_error(
    url: &str,
    context: &str,
    error: impl core::fmt::Display,
) -> ClientError {
    ClientError::Network(format!("{context} for URL: {url}: {error}"))
}

pub(crate) fn body_timeout_error(
    url: &str,
    limit: Duration,
    error: impl core::fmt::Display,
) -> ClientError {
    network_error(
        url,
        &format!("HTTP response body timed out after {limit:?}"),
        error,
    )
}
