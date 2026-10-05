//! Configuration management for a Muak-based system.

#![warn(missing_docs)]

extern crate alloc;

pub mod auth;
mod codec;
pub mod error;
pub mod permission;
pub mod system;
pub mod user;
pub mod version;

use crate::error::Result;

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
