//! CLI tool for creating Unified Kernel Images (UKI) for Linux on UEFI systems.

#[cfg(feature = "cli")]
mod cli {
    use std::path::PathBuf;

    use anyhow::{Context, Result};
    use clap::Parser;

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

        #[arg(long, help = "Path to raw LUKS key file")]
        luks: Option<PathBuf>,

        #[arg(short, long, help = "Output path for the generated UKI")]
        output: PathBuf,
    }

    pub fn run() -> Result<()> {
        let args = Args::parse();

        let luks_data = match &args.luks {
            Some(path) => Some(
                std::fs::read(path)
                    .with_context(|| format!("Failed to read LUKS key from {}", path.display()))?,
            ),
            None => None,
        };

        let buffer = yuki::build(
            &args.stub,
            &args.linux,
            &args.initrd,
            &args.cmdline,
            args.dtb.as_deref(),
            luks_data.as_deref(),
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
}

#[cfg(feature = "cli")]
fn main() {
    if let Err(e) = cli::run() {
        eprintln!("Error: {e:?}");
        std::process::exit(1);
    }
}
