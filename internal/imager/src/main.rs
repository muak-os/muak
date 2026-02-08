//! CLI tool to manage OCI images and initramfs generation.

#[cfg(feature = "cli")]
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
    },
}

/// Main entry point for the imager CLI.
fn main() -> Result<()> {
    let args = Cli::parse();

    match args.command {
        Command::Build {
            base,
            extension,
            output,
        } => {
            imager::build_initramfs(&base, &extension, &output)
                .context("Failed to build initramfs")?;
            println!(
                "Successfully created initramfs at {} ({} bytes)",
                output.display(),
                std::fs::metadata(&output)?.len()
            );
        }
        Command::Pull { image, output } => {
            imager::pull_image(&image, &output).context("Failed to pull image")?;
            println!("Successfully extracted image to {}", output.display());
        }
    }

    Ok(())
}
