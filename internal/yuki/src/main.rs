//! CLI tool for creating Unified Kernel Images (UKI) for Linux on UEFI systems.

use clap::Parser;
use std::path::PathBuf;
use yuki::build_uki;

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

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = Args::parse();

    let output_len = build_uki(
        &args.stub,
        &args.linux,
        &args.initrd,
        &args.cmdline,
        &args.output,
    )
    .map_err(|e| {
        eprintln!("Error: {}", e);
        e
    })?;

    println!(
        "Successfully created UKI at {} ({} bytes)",
        args.output.display(),
        output_len
    );

    Ok(())
}
