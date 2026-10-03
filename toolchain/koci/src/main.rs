//! CLI entry point for koci.

pub mod cli;

fn main() {
    std::process::exit(cli::run());
}
