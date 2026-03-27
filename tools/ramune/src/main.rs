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
            modules: PathBuf,

            #[arg(short, long)]
            file_contexts: Option<PathBuf>,

            #[arg(short, long)]
            output: PathBuf,
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

    /// Loads and parses a SELinux file_contexts file.
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
                modules,
                file_contexts,
                output,
            } => {
                let fc = file_contexts
                    .as_ref()
                    .map(|p| load_file_contexts(p))
                    .transpose()?;

                let config = ramune::CreateConfig {
                    init: &init,
                    rootfs_dir: &rootfs_dir,
                    modules: &modules,
                    file_contexts: fc.as_ref(),
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
                ramune::extend_initramfs(&base, &extension, &output)
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
