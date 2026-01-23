//! CLI tool to manage OCI images and initramfs generation.

#[cfg(feature = "cli")]
use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use std::path::PathBuf;

#[derive(Parser)]
#[command(name = env!("CARGO_PKG_NAME"))]
#[command(about = env!("CARGO_PKG_DESCRIPTION"))]
struct Args {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Build a custom initramfs with extensions
    Build {
        #[arg(short, long)]
        base: PathBuf,

        #[arg(short, long)]
        extension: Vec<String>,

        #[arg(short, long)]
        output: PathBuf,
    },
    /// Pull and extract an OCI image
    Pull {
        #[arg(short, long)]
        image: String,

        #[arg(short, long)]
        output: PathBuf,
    },
}

fn main() -> Result<()> {
    let args = Args::parse();

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
