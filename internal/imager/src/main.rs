//! CLI tool to manage OCI images and initramfs generation.

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
            extension: Vec<String>,

            #[arg(short, long)]
            output: PathBuf,
        },
        Pull {
            #[arg(short, long)]
            image: String,

            #[arg(short, long)]
            output: PathBuf,

            #[arg(long, value_name = "PATH")]
            cosign_key: Option<PathBuf>,
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
                imager::build_initramfs(&base, &extension, &output)
                    .await
                    .context("Failed to build initramfs")?;
                println!(
                    "Successfully created initramfs at {} ({} bytes)",
                    output.display(),
                    std::fs::metadata(&output)?.len()
                );
            }
            Command::Pull {
                image,
                output,
                cosign_key,
            } => {
                let key_contents = cosign_key
                    .map(|p| {
                        std::fs::read_to_string(&p).with_context(|| {
                            format!("Failed to read cosign public key from {}", p.display())
                        })
                    })
                    .transpose()?;

                imager::pull_image(&image, &output, key_contents.as_deref())
                    .await
                    .context("Failed to pull image")?;
                println!("Successfully extracted image to {}", output.display());
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
