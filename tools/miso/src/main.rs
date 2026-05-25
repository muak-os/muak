//! CLI entry point for miso — bootable image builder.

#[cfg(feature = "cli")]
use miso::cli;

#[cfg(feature = "cli")]
fn main() {
    std::process::exit(cli::run());
}
