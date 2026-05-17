//! Command-line interface for ramune.

use std::ffi::OsString;
use std::path::PathBuf;

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};

#[derive(Debug, Parser)]
#[command(name = env!("CARGO_PKG_NAME"))]
#[command(about = env!("CARGO_PKG_DESCRIPTION"))]
#[command(version = env!("CARGO_PKG_VERSION"))]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
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

        #[arg(long, default_value_t = ::erofs::DEFAULT_ZSTD_COMPRESSION_LEVEL)]
        rootfs_compression_level: i32,
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

/// Parses command-line arguments and runs the requested command.
pub async fn run_with<I, T>(args: I) -> Result<String>
where
    I: IntoIterator<Item = T>,
    T: Into<OsString> + Clone,
{
    let args = Cli::try_parse_from(args)?;
    run_command(args.command).await
}

async fn run_command(command: Command) -> Result<String> {
    match command {
        Command::Create {
            init,
            rootfs_dir,
            file_contexts,
            output,
            compression_level,
            rootfs_compression_level,
        } => {
            let file_contexts = match file_contexts.as_ref() {
                Some(path) => {
                    let file = std::fs::File::open(path)
                        .context(format!("Failed to open file_contexts: {}", path.display()))?;
                    Some(
                        erofs::FileContexts::from_reader(file)
                            .context("Failed to parse file_contexts")?,
                    )
                }
                None => None,
            };

            let config = crate::CreateConfig {
                init: &init,
                rootfs_dir: &rootfs_dir,
                file_contexts: file_contexts.as_ref(),
                compression_level,
                rootfs_compression_level,
            };

            crate::create(&config, &output).context("Failed to create initramfs")?;

            Ok(format!(
                "Successfully created initramfs at {}",
                output.display()
            ))
        }
        Command::Extend {
            base,
            extension,
            output,
        } => {
            let extensions: Vec<(String, PathBuf)> = extension
                .iter()
                .map(|path| {
                    let name = match path.file_name() {
                        Some(name) => name.to_string_lossy().into_owned(),
                        None => path.display().to_string(),
                    };
                    (name, path.clone())
                })
                .collect();

            crate::extend(&base, &extensions, &output)
                .await
                .context("Failed to build initramfs")?;

            Ok(format!(
                "Successfully created initramfs at {}",
                output.display()
            ))
        }
    }
}
