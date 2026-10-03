//! CLI entry point for the catalog publisher.

#[cfg(feature = "cli")]
mod cli;

fn main() {
    std::process::exit(cli::run());
}
