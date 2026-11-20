//! Release catalog schemas and publisher.

#![warn(missing_docs)]
#![cfg_attr(
    feature = "cli",
    expect(
        clippy::print_stderr,
        reason = "release progress is reported on stderr until the logging facade lands"
    )
)]

#[cfg(feature = "cli")]
extern crate alloc;

pub mod error;
#[cfg(feature = "cli")]
pub mod progress;
pub mod schema;

#[cfg(feature = "cli")]
pub mod ops;
#[cfg(feature = "cli")]
pub mod repository;
