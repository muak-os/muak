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
        Iso {
            #[arg(short, long, help = "Path to the UKI .efi file")]
            uki: PathBuf,

            #[arg(short, long, help = "Path for the output .iso file")]
            output: PathBuf,

            #[arg(
                short,
                long,
                default_value = "x86_64",
                help = "Target architecture: x86_64 or aarch64"
            )]
            arch: String,
        },

        Img {
            #[arg(short, long, help = "Path to the UKI .efi file")]
            uki: PathBuf,

            #[arg(short, long, help = "Path for the output .img file")]
            output: PathBuf,

            #[arg(
                short,
                long = "blob",
                help = "Extra file as src:dst (e.g. ./start4.elf:START4.ELF)"
            )]
            blobs: Vec<String>,
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

    /// Parses a `src:dst` blob spec into `(src_path, dst_name)`.
    fn parse_blob(spec: &str) -> Result<(&str, &str)> {
        let (src, dst) = spec
            .split_once(':')
            .with_context(|| format!("Invalid blob spec '{spec}': expected src:dst"))?;
        anyhow::ensure!(!src.is_empty(), "Blob source path is empty in '{spec}'");
        anyhow::ensure!(
            !dst.is_empty(),
            "Blob destination name is empty in '{spec}'"
        );
        Ok((src, dst))
    }

    /// Reads a `src:dst` blob spec from disk, returning `(dst_name, file_data)`.
    fn load_blob(src: &str, dst: &str) -> Result<(String, Vec<u8>)> {
        let data = std::fs::read(src).with_context(|| format!("Failed to read blob '{src}'"))?;
        Ok((dst.to_owned(), data))
    }

    pub fn run() -> Result<()> {
        let args = Args::parse();

        match args.command {
            Command::Iso { uki, output, arch } => {
                let arch = parse_arch(&arch)?;
                let uki_bytes = std::fs::read(&uki)
                    .with_context(|| format!("Failed to read {}", uki.display()))?;
                let iso = miso::build_iso(&uki_bytes, arch).context("Failed to build ISO")?;
                std::fs::write(&output, &iso)
                    .with_context(|| format!("Failed to write {}", output.display()))?;
                println!("ISO written to {} ({} bytes)", output.display(), iso.len());
                Ok(())
            }
            Command::Img { uki, output, blobs } => {
                let uki_bytes = std::fs::read(&uki)
                    .with_context(|| format!("Failed to read {}", uki.display()))?;

                let blob_specs: Vec<(&str, &str)> =
                    blobs.iter().map(|s| parse_blob(s)).collect::<Result<_>>()?;

                let blob_data: Vec<(String, Vec<u8>)> = blob_specs
                    .iter()
                    .map(|&(src, dst)| load_blob(src, dst))
                    .collect::<Result<_>>()?;

                let blob_refs: Vec<(&str, &[u8])> = blob_data
                    .iter()
                    .map(|(dst, data)| (dst.as_str(), data.as_slice()))
                    .collect();

                let img = miso::build_img(&uki_bytes, &blob_refs)
                    .context("Failed to build disk image")?;
                std::fs::write(&output, &img)
                    .with_context(|| format!("Failed to write {}", output.display()))?;
                println!(
                    "Image written to {} ({} bytes)",
                    output.display(),
                    img.len()
                );
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
