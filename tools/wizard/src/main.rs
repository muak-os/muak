#[cfg(feature = "cli")]
use wizard::cli;

#[cfg(feature = "cli")]
#[tokio::main]
async fn main() {
    std::process::exit(cli::run().await);
}
