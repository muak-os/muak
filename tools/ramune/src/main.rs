//! CLI tool for creating and extending initramfs images.

#[cfg(feature = "cli")]
mod cli {
    use std::path::{Path, PathBuf};

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
        Create {
            #[arg(short, long)]
            init: PathBuf,

            #[arg(short, long)]
            rootfs_dir: PathBuf,

            #[arg(short, long)]
            file_contexts: Option<PathBuf>,

            #[arg(short, long)]
            output: PathBuf,

            #[arg(long, default_value_t = 19)]
            compression_level: i32,
        },
        Extend {
            #[arg(short, long)]
            base: PathBuf,

            #[arg(short, long)]
            extension: Vec<PathBuf>,

            #[arg(short, long)]
            output: PathBuf,
        },
    }

    /// Derives a logical name for an extension from its directory path.
    fn ext_name(p: &Path) -> String {
        p.file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| p.display().to_string())
    }

    /// Load and parse SELinux file contexts from a file.
    fn load_file_contexts(path: &Path) -> Result<erofs::FileContexts> {
        let file = std::fs::File::open(path)
            .with_context(|| format!("Failed to open file_contexts: {}", path.display()))?;
        erofs::FileContexts::from_reader(file).context("Failed to parse file_contexts")
    }

    pub async fn run() -> Result<()> {
        let args = Cli::parse();

        match args.command {
            Command::Create {
                init,
                rootfs_dir,
                file_contexts,
                output,
                compression_level,
            } => {
                let fc = file_contexts
                    .as_ref()
                    .map(|p| load_file_contexts(p))
                    .transpose()?;

                let config = ramune::CreateConfig {
                    init: &init,
                    rootfs_dir: &rootfs_dir,
                    file_contexts: fc.as_ref(),
                    compression_level,
                };

                ramune::create_initramfs(&config, &output).context("Failed to create initramfs")?;

                println!(
                    "Successfully created initramfs at {} ({} bytes)",
                    output.display(),
                    std::fs::metadata(&output)?.len()
                );
            }
            Command::Extend {
                base,
                extension,
                output,
            } => {
                let ext_pairs: Vec<(String, PathBuf)> =
                    extension.iter().map(|p| (ext_name(p), p.clone())).collect();
                ramune::extend_initramfs(&base, &ext_pairs, &output)
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
