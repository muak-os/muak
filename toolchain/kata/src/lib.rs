//! Release catalog schemas and publisher.

#![warn(missing_docs)]

#[cfg(feature = "publisher")]
extern crate alloc;

#[cfg(feature = "publisher")]
pub mod compose;
pub mod error;
#[cfg(feature = "publisher")]
pub mod progress;
#[cfg(feature = "publisher")]
pub mod publish;
#[cfg(feature = "publisher")]
pub mod remove;
#[cfg(feature = "publisher")]
pub mod repository;
pub mod schema;
#[cfg(feature = "publisher")]
pub mod verify;
