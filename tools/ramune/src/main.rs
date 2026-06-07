//! CLI tool for creating and extending initramfs images.

#[cfg(feature = "cli")]
use ramune::cli;

#[cfg(feature = "cli")]
fn main() {
    std::process::exit(cli::run());
}
