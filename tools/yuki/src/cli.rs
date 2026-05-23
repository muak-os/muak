//! Command-line interface for yuki.

use std::ffi::OsString;
use std::path::PathBuf;

use anyhow::{Context, Result};
use clap::Parser;

#[derive(Debug, Parser)]
#[command(name = env!("CARGO_PKG_NAME"))]
#[command(about = env!("CARGO_PKG_DESCRIPTION"))]
#[command(version = env!("CARGO_PKG_VERSION"))]
struct Cli {
    #[arg(short, long, help = "Path to the EFI stub PE file")]
    stub: PathBuf,

    #[arg(short = 'l', long, help = "Path to the Linux kernel image")]
    linux: PathBuf,

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

    #[arg(long, help = "Optional LUKS key file to include in the UKI")]
    luks: Option<PathBuf>,

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
    run(args)
}

fn run(args: Cli) -> Result<String> {
    let luks_data = match &args.luks {
        Some(path) => Some(
            std::fs::read(path)
                .with_context(|| format!("Failed to read LUKS key from {}", path.display()))?,
        ),
        None => None,
    };

    let buffer = crate::build(&crate::Components {
        stub: args.stub,
        kernel: args.linux,
        initramfs: args.initrd,
        cmdline: args.cmdline,
        dtb: args.dtb,
        luks_key: luks_data,
    })
    .context("Failed to create UKI")?;

    std::fs::write(&args.output, &buffer)
        .with_context(|| format!("Failed to write UKI to {}", args.output.display()))?;

    Ok(format!(
        "Successfully created UKI at {} ({} bytes)",
        args.output.display(),
        buffer.len()
    ))
}

#[cfg(all(test, feature = "cli"))]
mod tests {
    use super::*;

    #[test]
    fn run_with_missing_required_argument_returns_clap_error() {
        // ARRANGE & ACT
        let error = run_with(["yuki"]).expect_err("missing required args should error");

        // ASSERT
        assert!(error.downcast_ref::<clap::Error>().is_some());
    }
}
