//! CLI tool for creating Unified Kernel Images (UKI) for Linux on UEFI systems.

use anyhow::{Context, Result};
use clap::Parser;
use std::path::PathBuf;
use yuki::build;

#[derive(Parser, Debug)]
#[command(name = env!("CARGO_PKG_NAME"))]
#[command(about = env!("CARGO_PKG_DESCRIPTION"))]
struct Args {
    #[arg(short, long)]
    stub: PathBuf,

    #[arg(short, long)]
    linux: PathBuf,

    #[arg(short, long)]
    initrd: PathBuf,

    #[arg(short, long)]
    cmdline: PathBuf,

    #[arg(short, long)]
    output: PathBuf,
}

fn main() -> Result<()> {
    let args = Args::parse();

    let output_len = build(
        &args.stub,
        &args.linux,
        &args.initrd,
        &args.cmdline,
        &args.output,
    )
    .context("Failed to create UKI")?;

    println!(
        "Successfully created UKI at {} ({} bytes)",
        args.output.display(),
        output_len
    );

    Ok(())
}
