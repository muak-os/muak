//! CLI entry point for miso — bootable image builder.

#[cfg(feature = "cli")]
mod cli {
    use std::path::PathBuf;

    use anyhow::{Context, Result};
    use clap::{Parser, Subcommand};

    /// Top-level CLI arguments.
    #[derive(Parser, Debug)]
    #[command(name = env!("CARGO_PKG_NAME"))]
    #[command(about = env!("CARGO_PKG_DESCRIPTION"))]
    struct Args {
        #[command(subcommand)]
        command: Command,
    }

    /// Available subcommands.
    #[derive(Subcommand, Debug)]
    enum Command {
        /// Build a bootable ISO 9660 image from a UKI EFI binary.
        Iso {
            #[arg(short, long, help = "Path to the UKI .efi file")]
            uki: PathBuf,

            #[arg(short, long, help = "Path for the output .iso file")]
            output: PathBuf,

            #[arg(
                short,
                long,
                default_value = "MUAK",
                help = "ISO volume label (up to 32 chars)"
            )]
            label: String,

            #[arg(
                short,
                long,
                default_value = "x86_64",
                help = "Target architecture: x86_64 or aarch64"
            )]
            arch: String,
        },
    }

    /// Parses the architecture string into a `miso::Arch` value.
    fn parse_arch(arch: &str) -> Result<miso::Arch> {
        match arch {
            "x86_64" => Ok(miso::Arch::X86_64),
            "aarch64" => Ok(miso::Arch::Aarch64),
            other => anyhow::bail!(
                "Unsupported architecture: '{}'. Use x86_64 or aarch64.",
                other
            ),
        }
    }

    pub fn run() -> Result<()> {
        let args = Args::parse();

        match args.command {
            Command::Iso {
                uki,
                output,
                label,
                arch,
            } => {
                let arch = parse_arch(&arch)?;
                let uki_bytes = std::fs::read(&uki)
                    .with_context(|| format!("Failed to read {}", uki.display()))?;
                let iso =
                    miso::build_iso(&uki_bytes, arch, &label).context("Failed to build ISO")?;
                std::fs::write(&output, &iso)
                    .with_context(|| format!("Failed to write {}", output.display()))?;
                println!("ISO written to {} ({} bytes)", output.display(), iso.len());
                Ok(())
            }
        }
    }
}

#[cfg(feature = "cli")]
fn main() {
    if let Err(e) = cli::run() {
        eprintln!("Error: {e:?}");
        std::process::exit(1);
    }
}
