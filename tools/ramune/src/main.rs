//! CLI tool for creating and extending initramfs images.

pub mod cli;

fn main() {
    std::process::exit(cli::run_with(std::env::args_os()));
}
