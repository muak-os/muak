//! CLI tool for creating and extending initramfs images.

#[cfg(feature = "cli")]
#[tokio::main]
async fn main() {
    let result = ramune::cli::run_with(std::env::args_os()).await;
    let error = match result {
        Ok(message) => {
            println!("{message}");
            return;
        }
        Err(error) => error,
    };

    if let Some(clap_error) = error.downcast_ref::<clap::Error>() {
        let _ = clap_error.print();
        std::process::exit(clap_error.exit_code());
    }

    eprintln!("Error: {error:?}");
    std::process::exit(1);
}
