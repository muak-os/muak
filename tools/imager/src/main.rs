#[cfg(feature = "cli")]
use imager::cli;

#[cfg(feature = "cli")]
#[tokio::main]
async fn main() {
    std::process::exit(cli::run().await);
}
