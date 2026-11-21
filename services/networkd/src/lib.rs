//! Network daemon library.

#![expect(
    clippy::print_stdout,
    clippy::print_stderr,
    reason = "service output is captured per service by PID 1; the logging facade is pending"
)]

extern crate alloc;

#[cfg(not(any(feature = "ipv4", feature = "ipv6")))]
compile_error!("enable at least one of the ipv4 or ipv6 features");

pub mod dns;
pub mod statemachine;
pub mod supervisor;
