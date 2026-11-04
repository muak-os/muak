//! Error construction for registry transport failures.

use core::time::Duration;

use crate::error::ClientError;

pub(crate) fn error(url: &str, context: &str, cause: impl core::fmt::Display) -> ClientError {
    ClientError::Network(format!("{context} for URL: {url}: {cause}"))
}

pub(crate) fn body_timeout(
    url: &str,
    limit: Duration,
    cause: impl core::fmt::Display,
) -> ClientError {
    error(
        url,
        &format!("HTTP response body timed out after {limit:?}"),
        cause,
    )
}
