//! Command-line interface for yuki.

use std::ffi::OsString;
use std::fs::File;
use std::path::PathBuf;

use anyhow::{Context as _, Result};
use clap::Parser;

use crate::builder::Builder;
use crate::layout;

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

    #[arg(
        short = 'd',
        long,
        help = "Optional device tree blob to include in the UKI"
    )]
    dtb: Option<PathBuf>,

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
    let dtb_size = args
        .dtb
        .as_ref()
        .map(|path| {
            std::fs::metadata(path)
                .with_context(|| format!("Failed to read DTB from {}", path.display()))
                .map(|meta| meta.len())
        })
        .transpose()?;

    let mut stub_file = File::open(&args.stub)
        .with_context(|| format!("Failed to read EFI stub from {}", args.stub.display()))?;
    let mut kernel = File::open(&args.kernel)
        .with_context(|| format!("Failed to read kernel from {}", args.kernel.display()))?;
    let mut initrd = File::open(&args.initrd)
        .with_context(|| format!("Failed to read initramfs from {}", args.initrd.display()))?;
    let mut cmdline = File::open(&args.cmdline)
        .with_context(|| format!("Failed to read cmdline from {}", args.cmdline.display()))?;
    let mut dtb = args
        .dtb
        .as_ref()
        .map(|path| {
            File::open(path).with_context(|| format!("Failed to read DTB from {}", path.display()))
        })
        .transpose()?;

    let (_layout, state) = layout::compute(
        &mut stub_file,
        stub_size,
        cmdline_size,
        kernel_size,
        initrd_size,
        dtb_size,
    )
    .context("Failed to compute UKI layout")?;

    let mut output = File::create(&args.output)
        .with_context(|| format!("Failed to write UKI to {}", args.output.display()))?;

    let builder = Builder::new(state, &mut output);
    let builder = builder
        .add_stub(&mut stub_file)
        .context("Failed to add stub")?;
    let builder = builder
        .add_cmdline(&mut cmdline)
        .context("Failed to add cmdline")?;

    let builder = if let Some(dtb_file) = dtb.as_mut() {
        builder.add_dtb(dtb_file).context("Failed to add DTB")?
    } else {
        builder
    };

    let builder = builder
        .add_kernel(&mut kernel)
        .context("Failed to add kernel")?;
    let builder = builder
        .add_initramfs(&mut initrd)
        .context("Failed to add initramfs")?;
    builder.finish().context("Failed to finalize UKI")?;

    Ok(format!(
        "Successfully created UKI at {} ({} bytes)",
        args.output.display(),
        output
            .metadata()
            .context("Failed to read output metadata")?
            .len()
    ))
}
