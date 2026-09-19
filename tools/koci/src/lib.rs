//! OCI image pulling, annotation, and signing.

#![warn(missing_docs)]

extern crate alloc;

#[cfg(any(feature = "sign", feature = "annotate"))]
pub mod annotations;
pub mod arch;
pub mod error;
#[cfg(feature = "merge")]
pub mod merge;
#[cfg(feature = "pull")]
pub mod pull;
#[cfg(feature = "push")]
pub mod push;
#[cfg(feature = "registry")]
pub mod registry;
#[cfg(feature = "runtime")]
mod runtime;
#[cfg(any(feature = "pull", feature = "sign"))]
pub mod signature;
