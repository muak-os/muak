//! CLI entry point for koci.

#[cfg(feature = "cli")]
use koci::cli;

#[cfg(feature = "cli")]
fn main() {
    std::process::exit(cli::run());
}
