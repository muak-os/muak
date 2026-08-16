#[cfg(feature = "cli")]
use wizard::cli;

#[cfg(feature = "cli")]
fn main() {
    std::process::exit(cli::run());
}
