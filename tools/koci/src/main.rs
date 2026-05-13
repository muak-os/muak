//! CLI tool for OCI image pulling and manifest signing.

#[cfg(feature = "cli")]
mod cli {
    use std::path::PathBuf;

    use anyhow::{Context, Result};
    use clap::{Parser, Subcommand};

    fn host_oci_arch() -> &'static str {
        match std::env::consts::ARCH {
            "aarch64" => "arm64",
            "x86_64" => "amd64",
            other => other,
        }
    }

    #[derive(Parser)]
    #[command(name = env!("CARGO_PKG_NAME"))]
    #[command(about = env!("CARGO_PKG_DESCRIPTION"))]
    struct Cli {
        #[command(subcommand)]
        command: Command,
    }

    #[derive(Subcommand)]
    enum Command {
        Pull {
            #[arg(short, long)]
            image: String,

            #[arg(long)]
            arch: Option<String>,

            #[arg(short, long)]
            output: PathBuf,

            #[arg(long, value_name = "PATH")]
            pub_key: Option<PathBuf>,
        },
        Sign {
            #[arg(short, long)]
            image: String,
            #[arg(long, value_name = "PATH")]
            key: PathBuf,
        },
    }

    fn read_key_file(path: &std::path::Path) -> Result<String> {
        std::fs::read_to_string(path)
            .with_context(|| format!("Failed to read key from {}", path.display()))
    }

    pub async fn run() -> Result<()> {
        let args = Cli::parse();

        match args.command {
            Command::Pull {
                image,
                arch,
                output,
                pub_key,
            } => {
                let key_contents = pub_key.map(|p| read_key_file(&p)).transpose()?;
                let arch = arch.unwrap_or_else(|| host_oci_arch().to_string());

                koci::pull(&image, &arch, &output, key_contents.as_deref())
                    .await
                    .context("Failed to pull image")?;
                println!("Successfully extracted image to {}", output.display());
            }
            Command::Sign { image, key } => {
                let privkey_pem = read_key_file(&key)?;

                koci::sign(&image, &privkey_pem)
                    .await
                    .context("Failed to sign image")?;
                println!("Successfully signed {}", image);
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
