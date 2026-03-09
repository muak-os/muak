//! # sysconfig
//!
//! Configuration management for a Muak-based system.
//!
//! System config (`/run/state/config.toml`) is immutable after boot and loaded
//! once into a `OnceLock`. Auth state (`/run/state/auth.toml`) is mutable and
//! reloaded from disk on every access via mtime checking.
//!
//! ## Example
//!
//! ```rust,ignore
//! use sysconfig::{init, host};
//!
//! fn main() -> Result<(), Box<dyn std::error::Error>> {
//!     init()?;
//!     let disk = &host().disk;
//!     Ok(())
//! }
//! ```

pub mod auth;
mod error;
pub mod permission;
mod system;
pub mod version;

pub use auth::{AUTH_EXTENSION, AUTH_PATH, AuthConfig, AuthUser, serialize as serialize_auth};
pub use error::{ConfigError, Result};
pub use permission::Permission;
pub use system::{
    CONFIG_EXTENSION, CONFIG_PATH, HostConfig, NetworkConfig, SystemConfig, VmConfig,
    load_from_path, parse_from_str, serialize, serialize_default,
};
pub use version::check_no_downgrade;

/// Initializes the system config and auth cache.
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
pub fn host() -> &'static HostConfig {
    &config().host
}

/// Returns the global network configuration.
///
/// # Panics
///
/// Panics if [`init()`] has not been called.
pub fn network() -> &'static NetworkConfig {
    &config().network
}

/// Returns the global VM configuration.
///
/// # Panics
///
/// Panics if [`init()`] has not been called.
pub fn vm() -> &'static VmConfig {
    &config().vm
}

/// Returns the current auth config, reloading from disk if the file changed.
///
/// # Panics
///
/// Panics if [`init()`] has not been called.
pub fn auth() -> std::sync::Arc<AuthConfig> {
    auth::auth()
}

/// Returns the current auth config, or `None` before [`init()`].
pub fn try_auth() -> Option<std::sync::Arc<AuthConfig>> {
    auth::try_auth()
}

/// Returns the global system configuration.
///
/// # Panics
///
/// Panics if [`init()`] has not been called.
pub fn config() -> &'static SystemConfig {
    system::CONFIG.get().expect("Config not initialized")
}

/// Returns the global system configuration, or `None` before [`init()`].
pub fn try_config() -> Option<&'static SystemConfig> {
    system::CONFIG.get()
}
