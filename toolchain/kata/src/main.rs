//! CLI entry point for the catalog publisher.

#![expect(
    clippy::print_stderr,
    reason = "command failures are reported on standard error"
)]

mod cli;

fn main() {
    std::process::exit(cli::run());
}
