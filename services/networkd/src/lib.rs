//! Network daemon library.

#![expect(
    clippy::print_stdout,
    clippy::print_stderr,
    reason = "service output is captured per service by PID 1; the logging facade is pending"
)]

extern crate alloc;

pub mod dns;
pub mod interface;
#[cfg(feature = "ipv6")]
pub mod slaac;
pub mod statemachine;
pub mod supervisor;
