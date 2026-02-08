//! CLI tool for creating Unified Kernel Images (UKI) for Linux on UEFI systems.

#[cfg(feature = "cli")]
use anyhow::{Context, Result};
use clap::Parser;
use std::path::PathBuf;
use yuki::build;

/// Command line arguments for the UKI builder
#[derive(Parser, Debug)]
#[command(name = env!("CARGO_PKG_NAME"))]
#[command(about = env!("CARGO_PKG_DESCRIPTION"))]
struct Args {
    #[arg(short, long, help = "Path to EFI stub file")]
    stub: PathBuf,

    #[arg(short, long, help = "Path to Linux kernel image")]
    linux: PathBuf,

    #[arg(short, long, help = "Path to initrd image")]
    initrd: PathBuf,

    #[arg(short, long, help = "Path to text file containing kernel command line")]
    cmdline: PathBuf,

    #[arg(short, long, help = "Path to device tree blob (optional, for ARM64)")]
    dtb: Option<PathBuf>,

    #[arg(short, long, help = "Output path for the generated UKI")]
    output: PathBuf,
}

/// Entry point for the UKI builder CLI
fn main() -> Result<()> {
    let args = Args::parse();

    let buffer = build(
        &args.stub,
        &args.linux,
        &args.initrd,
        &args.cmdline,
        args.dtb.as_deref(),
    )
    .context("Failed to create UKI")?;

    std::fs::write(&args.output, &buffer)
        .with_context(|| format!("Failed to write UKI to {}", args.output.display()))?;

    println!(
        "Successfully created UKI at {} ({} bytes)",
        args.output.display(),
        buffer.len()
    );

    Ok(())
}
