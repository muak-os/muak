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
//! use sysconfig::{init, system};
//!
//! fn main() -> Result<(), Box<dyn std::error::Error>> {
//!     init()?;
//!     let disk = &system().disk;
//!     Ok(())
//! }
//! ```

use std::path::Path;
use std::sync::OnceLock;

pub mod auth;
mod error;
mod host;
pub mod permission;

pub use auth::{AUTH_PATH, AuthConfig, AuthUser, serialize as serialize_auth};
pub use error::{ConfigError, Result};
pub use host::{
    HostConfig, NetworkConfig, SystemConfig, VmConfig, load_from_path, parse_from_str, serialize,
    serialize_default,
};
pub use permission::Permission;

pub const CONFIG_PATH: &str = "/run/state/config.toml";

static CONFIG: OnceLock<HostConfig> = OnceLock::new();

/// Initializes the global config and auth cache.
///
/// Loads system config from `CONFIG_PATH` (validating it) and bootstraps
/// the auth cache from `AUTH_PATH`. Must be called before any access.
pub fn init() -> Result<()> {
    let config = load_from_path(Path::new(CONFIG_PATH))?;
    config.validate()?;
    CONFIG
        .set(config)
        .map_err(|_| ConfigError::AlreadyInitialized)?;
    auth::init()?;
    Ok(())
}

/// Returns the global system configuration.
///
/// # Panics
///
/// Panics if [`init()`] has not been called.
pub fn system() -> &'static SystemConfig {
    &config().system
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

/// Returns the global host configuration.
///
/// # Panics
///
/// Panics if [`init()`] has not been called.
pub fn config() -> &'static HostConfig {
    CONFIG.get().expect("Config not initialized")
}

/// Returns the global host configuration, or `None` before [`init()`].
pub fn try_config() -> Option<&'static HostConfig> {
    CONFIG.get()
}
