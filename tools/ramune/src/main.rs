//! CLI tool for creating and extending initramfs images.

use ramune::cli;

fn main() {
    std::process::exit(cli::run_with(std::env::args_os()));
}
