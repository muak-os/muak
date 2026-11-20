//! CLI tool for creating and extending initramfs images.

#![expect(
    clippy::print_stdout,
    clippy::print_stderr,
    reason = "the created image path and the reported error are the command result"
)]

pub mod cli;

fn main() {
    std::process::exit(cli::run_with(std::env::args_os()));
}
