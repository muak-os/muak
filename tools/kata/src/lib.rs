//! Release catalog schemas and publisher.

#![warn(missing_docs)]

pub mod error;
pub mod schema;

#[cfg(feature = "cli")]
pub mod ops;
#[cfg(feature = "cli")]
pub mod repository;
