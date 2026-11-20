//! Release catalog schemas and publisher.

#![warn(missing_docs)]

#[cfg(feature = "cli")]
extern crate alloc;

#[cfg(feature = "cli")]
pub mod compose;
pub mod error;
#[cfg(feature = "cli")]
pub mod progress;
#[cfg(feature = "cli")]
pub mod publish;
#[cfg(feature = "cli")]
pub mod remove;
#[cfg(feature = "cli")]
pub mod repository;
pub mod schema;
#[cfg(feature = "cli")]
pub mod verify;
