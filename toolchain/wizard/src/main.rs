//! CLI entry point for wizard.

pub mod cli;

fn main() {
    std::process::exit(cli::run());
}
