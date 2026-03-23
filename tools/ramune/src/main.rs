//! CLI tool for building initramfs images from pre-extracted extension directories.

#[cfg(feature = "cli")]
mod cli {
    use std::path::PathBuf;

    use anyhow::{Context, Result};
    use clap::{Parser, Subcommand};

    #[derive(Parser)]
    #[command(name = env!("CARGO_PKG_NAME"))]
    #[command(about = env!("CARGO_PKG_DESCRIPTION"))]
    struct Cli {
        #[command(subcommand)]
        command: Command,
    }

    #[derive(Subcommand)]
    enum Command {
        Build {
            #[arg(short, long)]
            base: PathBuf,

            #[arg(short, long)]
            extension: Vec<PathBuf>,

            #[arg(short, long)]
            output: PathBuf,
        },
    }

    pub async fn run() -> Result<()> {
        let args = Cli::parse();

        match args.command {
            Command::Build {
                base,
                extension,
                output,
            } => {
                ramune::build_initramfs(&base, &extension, &output)
                    .await
                    .context("Failed to build initramfs")?;
                println!(
                    "Successfully created initramfs at {} ({} bytes)",
                    output.display(),
                    std::fs::metadata(&output)?.len()
                );
            }
        }

        Ok(())
    }
}

#[cfg(feature = "cli")]
#[tokio::main]
async fn main() {
    if let Err(e) = cli::run().await {
        eprintln!("Error: {e:?}");
        std::process::exit(1);
    }
}
