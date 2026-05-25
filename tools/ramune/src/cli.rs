//! Command-line interface for ramune.

use std::ffi::OsString;
use std::path::PathBuf;

use anyhow::{Context as _, Result};
use clap::{Parser, Subcommand};

use crate::{CreateConfig, ExtendConfig};

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

        #[arg(long, default_value_t = crate::DEFAULT_ZSTD_COMPRESSION_LEVEL)]
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

        #[arg(long, default_value_t = crate::DEFAULT_ZSTD_COMPRESSION_LEVEL)]
        compression_level: i32,

        #[arg(long, default_value_t = ::erofs::DEFAULT_ZSTD_COMPRESSION_LEVEL)]
        extension_compression_level: i32,
    },
}

/// Runs the CLI from a caller-provided argument iterator.
///
/// # Errors
///
/// Returns an error when argument parsing fails or when the requested command fails.
pub async fn run_from<I, T>(args: I) -> Result<()>
where
    I: IntoIterator<Item = T>,
    T: Into<OsString> + Clone,
{
    let args = Cli::parse_from(args);
    run_command(args.command).await
}

pub async fn run_with<I, T>(args: I) -> i32
where
    I: IntoIterator<Item = T>,
    T: Into<OsString> + Clone,
{
    match run_from(args).await {
        Ok(()) => 0,
        Err(error) => {
            eprintln!("Error: {error:?}");
            1
        }
    }
}

#[must_use]
pub async fn run() -> i32 {
    run_with(std::env::args_os()).await
}

fn initramfs_size(output: &std::path::Path) -> Result<u64> {
    Ok(std::fs::metadata(output)
        .with_context(|| format!("Failed to read initramfs metadata: {}", output.display()))?
        .len())
}

fn extension_name(path: &std::path::Path) -> String {
    match path.file_name() {
        Some(name) => name.to_string_lossy().into_owned(),
        None => path.display().to_string(),
    }
}

async fn run_command(command: Command) -> Result<()> {
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

            let config = CreateConfig {
                init: &init,
                rootfs_dir: &rootfs_dir,
                file_contexts: file_contexts.as_ref(),
                compression_level,
                rootfs_compression_level,
            };

            crate::create(&config, &output).context("Failed to create initramfs")?;
            let size = initramfs_size(&output)?;

            println!(
                "Successfully created initramfs at {} ({} bytes)",
                output.display(),
                size
            );

            Ok(())
        }
        Command::Extend {
            base,
            extension,
            output,
            compression_level,
            extension_compression_level,
        } => {
            let extensions: Vec<(String, PathBuf)> = extension
                .iter()
                .map(|path| (extension_name(path), path.clone()))
                .collect();

            let config = ExtendConfig {
                base: &base,
                extensions: &extensions,
                compression_level,
                extension_compression_level,
            };

            crate::extend(&config, &output)
                .await
                .context("Failed to build initramfs")?;

            println!("Successfully created initramfs at {}", output.display());

            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::*;

    #[test]
    fn initramfs_size_missing_output_reports_path() {
        // ARRANGE
        let path = Path::new("/nonexistent/initramfs.img");

        // ACT
        let error = initramfs_size(path).expect_err("missing file should error");

        // ASSERT
        assert!(
            error
                .to_string()
                .contains("Failed to read initramfs metadata: /nonexistent/initramfs.img")
        );
    }
}
