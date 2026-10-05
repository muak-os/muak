//! Configuration management for a Muak-based system.

#![warn(missing_docs)]

extern crate alloc;

mod auth;
mod codec;
mod error;
pub mod permission;
mod system;
pub mod user;
pub mod version;

pub use auth::{
    AUTH_EXTENSION, AUTH_PATH, AuthConfig, AuthUser, load_from_path as load_auth_from_path,
    parse as parse_auth, serialize as serialize_auth,
};
pub use error::{ConfigError, Result};
pub use permission::Permission;
pub use system::*;
pub use user::{ClientConfig, Credentials, PendingEnrollment, ServerContext};
pub use version::{CompatibilityStatus, check_compatibility, check_no_downgrade, parse_release};

/// Initializes the system config and auth cache.
///
/// # Errors
///
/// Returns an error when loading or validating the system config or auth
/// state fails, or when either was already initialized.
pub fn init() -> Result<()> {
    system::init()?;
    auth::init()?;

    Ok(())
}

/// Returns the global host configuration.
///
/// # Panics
///
/// Panics if [`init()`] has not been called.
#[must_use]
pub fn host() -> &'static HostConfig {
    &config().host
}

/// Returns the global network configuration.
///
/// # Panics
///
/// Panics if [`init()`] has not been called.
#[must_use]
pub fn network() -> &'static NetworkConfig {
    &config().network
}

/// Returns the global VM configuration.
///
/// # Panics
///
/// Panics if [`init()`] has not been called.
#[must_use]
pub fn vm() -> &'static VmConfig {
    &config().vm
}

/// Returns the current auth config, reloading from disk if the file changed.
///
/// # Panics
///
/// Panics if [`init()`] has not been called.
#[must_use]
pub fn auth() -> alloc::sync::Arc<AuthConfig> {
    auth::try_auth().unwrap_or_else(|| panic!("Auth not initialized"))
}

/// Returns the current auth config, or `None` before [`init()`].
#[must_use]
pub fn try_auth() -> Option<alloc::sync::Arc<AuthConfig>> {
    auth::try_auth()
}

/// Returns the global system configuration.
///
/// # Panics
///
/// Panics if [`init()`] has not been called.
#[must_use]
pub fn config() -> &'static SystemConfig {
    try_config().unwrap_or_else(|| panic!("Config not initialized"))
}

/// Returns the global system configuration, or `None` before [`init()`].
pub fn try_config() -> Option<&'static SystemConfig> {
    system::CONFIG.get()
}
