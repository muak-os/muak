//! CLI entry point for the catalog publisher.

#![expect(
    clippy::print_stdout,
    clippy::print_stderr,
    reason = "the catalog command result is reported on the console"
)]

#[cfg(feature = "cli")]
mod cli;

fn main() {
    std::process::exit(cli::run());
}
