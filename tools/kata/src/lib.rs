//! Release catalog schemas and publisher.

#![warn(missing_docs)]

extern crate alloc;

pub mod error;
pub mod schema;

#[cfg(feature = "cli")]
pub mod ops;
#[cfg(feature = "cli")]
pub mod repository;
