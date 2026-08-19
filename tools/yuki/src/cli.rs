//! Command-line interface for yuki.

use std::ffi::OsString;
use std::fs::File;
use std::path::PathBuf;

use anyhow::{Context as _, Result};
use clap::Parser;

use crate::prepare;
use crate::probe;
use crate::write::{self, Input};

#[derive(Debug, Parser)]
#[command(name = env!("CARGO_PKG_NAME"))]
#[command(about = env!("CARGO_PKG_DESCRIPTION"))]
#[command(version = env!("CARGO_PKG_VERSION"))]
struct Cli {
    #[arg(short, long, help = "Path to the EFI stub PE file")]
    stub: PathBuf,

    #[arg(short = 'l', long, help = "Path to the Linux kernel image")]
    kernel: PathBuf,

    #[arg(short = 'i', long, help = "Path to the initramfs image")]
    initrd: PathBuf,

    #[arg(
        short = 'c',
        long,
        help = "Path to the text file containing the kernel command line"
    )]
    cmdline: PathBuf,

    #[arg(short, long)]
    output: PathBuf,
}

/// Parses command-line arguments and builds the requested UKI.
///
/// # Errors
///
/// Returns an error if argument parsing fails.
pub fn run_with<I, T>(args: I) -> Result<String>
where
    I: IntoIterator<Item = T>,
    T: Into<OsString> + Clone,
{
    let args = Cli::try_parse_from(args)?;
    run(&args)
}

fn run(args: &Cli) -> Result<String> {
    let stub_size = std::fs::metadata(&args.stub)
        .with_context(|| format!("Failed to read EFI stub from {}", args.stub.display()))?
        .len();
    let cmdline_size = std::fs::metadata(&args.cmdline)
        .with_context(|| format!("Failed to read cmdline from {}", args.cmdline.display()))?
        .len();
    let kernel_size = std::fs::metadata(&args.kernel)
        .with_context(|| format!("Failed to read kernel from {}", args.kernel.display()))?
        .len();
    let initrd_size = std::fs::metadata(&args.initrd)
        .with_context(|| format!("Failed to read initramfs from {}", args.initrd.display()))?
        .len();

    let mut stub_file = File::open(&args.stub)
        .with_context(|| format!("Failed to read EFI stub from {}", args.stub.display()))?;
    let mut kernel = File::open(&args.kernel)
        .with_context(|| format!("Failed to read kernel from {}", args.kernel.display()))?;
    let mut initrd = File::open(&args.initrd)
        .with_context(|| format!("Failed to read initramfs from {}", args.initrd.display()))?;
    let mut cmdline = File::open(&args.cmdline)
        .with_context(|| format!("Failed to read cmdline from {}", args.cmdline.display()))?;

    let manifest = prepare::prepare(
        probe::probe(&mut stub_file).context("Failed to compute UKI layout")?,
        stub_size,
        cmdline_size,
        kernel_size,
        initrd_size,
    )
    .context("Failed to compute UKI layout")?;

    let mut output = File::create(&args.output)
        .with_context(|| format!("Failed to write UKI to {}", args.output.display()))?;

    write::write(
        &manifest,
        &mut stub_file,
        Input {
            reader: &mut cmdline,
            size: cmdline_size,
        },
        Input {
            reader: &mut kernel,
            size: kernel_size,
        },
        Input {
            reader: &mut initrd,
            size: initrd_size,
        },
        &mut output,
    )
    .context("Failed to write UKI")?;

    Ok(format!(
        "Successfully created UKI at {} ({} bytes)",
        args.output.display(),
        output
            .metadata()
            .context("Failed to read output metadata")?
            .len()
    ))
}
