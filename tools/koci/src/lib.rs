//! OCI image pulling, annotation, and signing.

#![warn(missing_docs)]

extern crate alloc;

pub mod annotations;
pub mod arch;
mod digest;
pub mod error;
mod image;
pub mod merge;
pub mod pull;
pub mod push;
mod registry;
mod runtime;
