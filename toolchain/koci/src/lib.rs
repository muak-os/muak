//! OCI image pulling, annotation, and signing.

#![warn(missing_docs)]

extern crate alloc;

#[cfg(feature = "sign")]
pub mod annotations;
pub mod arch;
#[cfg(feature = "copy")]
pub mod copy;
pub mod error;
#[cfg(feature = "push")]
mod image;
#[cfg(feature = "merge")]
pub mod merge;
pub mod progress;
#[cfg(feature = "pull")]
pub mod pull;
#[cfg(feature = "push")]
pub mod push;
#[cfg(feature = "registry")]
pub mod registry;
#[cfg(all(feature = "pull", feature = "push"))]
pub mod repack;
#[cfg(feature = "registry")]
mod runtime;
#[cfg(any(feature = "pull", feature = "sign"))]
pub mod signature;
#[cfg(feature = "stargz")]
pub mod stargz;
