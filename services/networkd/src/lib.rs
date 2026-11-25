//! Network daemon library.

#![expect(
    clippy::print_stdout,
    reason = "service output is captured per service by PID 1; the logging facade is pending"
)]

pub mod config;
pub mod dns;
mod logging;
mod notices;
pub mod service;
