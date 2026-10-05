//! CLI entry point for the catalog publisher.

extern crate alloc;

#[cfg(feature = "cli")]
mod cli;

fn main() {
    std::process::exit(cli::run());
}
