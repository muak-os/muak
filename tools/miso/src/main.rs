//! CLI entry point for miso — bootable image builder.

#[cfg(feature = "cli")]
mod cli {
    use std::fs::File;
    use std::path::PathBuf;

    use anyhow::{Context, Result, bail, ensure};
    use clap::{Parser, Subcommand};
    use esp::{Arch, EspFile, EspSpec};

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

            #[arg(short, long = "file", help = "Extra file as src:dst/path")]
            files: Vec<String>,
        },

        Img {
            #[arg(short, long, help = "Path to the UKI .efi file")]
            uki: PathBuf,

            #[arg(short, long, help = "Path for the output .img file")]
            output: PathBuf,

            #[arg(
                short,
                long,
                default_value = "aarch64",
                help = "Target architecture: x86_64 or aarch64"
            )]
            arch: String,

            #[arg(short, long = "file", help = "Extra file as src:dst/path")]
            files: Vec<String>,
        },
    }

    /// Parses the architecture string into a `miso::Arch` value.
    pub(super) fn parse_arch(arch: &str) -> Result<Arch> {
        match arch {
            "x86_64" => Ok(Arch::X86_64),
            "aarch64" => Ok(Arch::Aarch64),
            other => bail!(
                "Unsupported architecture: '{}'. Use x86_64 or aarch64.",
                other
            ),
        }
    }

    /// Parses a `src:dst` file spec into `(src_path, dst_path)`.
    pub(super) fn parse_file_spec(spec: &str) -> Result<(&str, &str)> {
        let (src, dst) = spec
            .split_once(':')
            .with_context(|| format!("Invalid file spec '{spec}': expected src:dst"))?;
        ensure!(!src.is_empty(), "File source path is empty in '{spec}'");
        ensure!(
            !dst.is_empty(),
            "File destination path is empty in '{spec}'"
        );
        Ok((src, dst))
    }

    /// Loads extra file entries from `src:dst` specs.
    pub(super) fn load_file_entries(specs: &[String]) -> Result<Vec<EspFile>> {
        specs
            .iter()
            .map(|s| {
                let (src, dst) = parse_file_spec(s)?;
                let data =
                    std::fs::read(src).with_context(|| format!("Failed to read file '{src}'"))?;
                Ok(EspFile {
                    path: dst.to_owned(),
                    data,
                })
            })
            .collect()
    }

    pub fn run() -> Result<()> {
        let args = Args::parse();

        match args.command {
            Command::Iso {
                uki,
                output,
                arch,
                files,
            } => {
                let arch = parse_arch(&arch)?;
                let uki_bytes = std::fs::read(&uki)
                    .with_context(|| format!("Failed to read {}", uki.display()))?;
                let extra_files = load_file_entries(&files)?;
                let spec = EspSpec::with_uki(arch, uki_bytes, extra_files);
                let mut file = File::create(&output)
                    .with_context(|| format!("Failed to create {}", output.display()))?;
                miso::build_iso(&spec, &mut file).context("Failed to build ISO")?;
                let size = file
                    .metadata()
                    .with_context(|| format!("Failed to stat {}", output.display()))?
                    .len();
                println!("ISO written to {} ({} bytes)", output.display(), size);
                Ok(())
            }
            Command::Img {
                uki,
                output,
                arch,
                files,
            } => {
                let arch = parse_arch(&arch)?;
                let uki_bytes = std::fs::read(&uki)
                    .with_context(|| format!("Failed to read {}", uki.display()))?;
                let extra_files = load_file_entries(&files)?;
                let spec = EspSpec::with_uki(arch, uki_bytes, extra_files);
                let mut file = File::create(&output)
                    .with_context(|| format!("Failed to create {}", output.display()))?;
                miso::build_img(&spec, &mut file).context("Failed to build disk image")?;
                let size = file
                    .metadata()
                    .with_context(|| format!("Failed to stat {}", output.display()))?
                    .len();
                println!("Image written to {} ({} bytes)", output.display(), size);
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

#[cfg(all(test, feature = "cli"))]
mod tests {
    use std::io::Write as _;

    use miso::Arch;
    use tempfile::NamedTempFile;

    use super::cli::{load_file_entries, parse_arch, parse_file_spec};

    #[test]
    fn parse_arch_x86_64() {
        // ARRANGE / ACT / ASSERT
        assert_eq!(parse_arch("x86_64").unwrap(), Arch::X86_64);
    }

    #[test]
    fn parse_arch_aarch64() {
        // ARRANGE / ACT / ASSERT
        assert_eq!(parse_arch("aarch64").unwrap(), Arch::Aarch64);
    }

    #[test]
    fn parse_arch_unknown_returns_error() {
        // ARRANGE / ACT / ASSERT
        assert!(parse_arch("riscv64").is_err());
    }

    #[test]
    fn parse_file_spec_valid() {
        // ARRANGE / ACT
        let (src, dst) = parse_file_spec("src/file.dat:dst/path.dat").unwrap();

        // ASSERT
        assert_eq!(src, "src/file.dat");
        assert_eq!(dst, "dst/path.dat");
    }

    #[test]
    fn parse_file_spec_missing_colon_returns_error() {
        // ARRANGE / ACT / ASSERT
        assert!(parse_file_spec("nodivider").is_err());
    }

    #[test]
    fn parse_file_spec_empty_src_returns_error() {
        // ARRANGE / ACT / ASSERT
        assert!(parse_file_spec(":dst").is_err());
    }

    #[test]
    fn parse_file_spec_empty_dst_returns_error() {
        // ARRANGE / ACT / ASSERT
        assert!(parse_file_spec("src:").is_err());
    }

    #[test]
    fn load_file_entries_empty_returns_empty_vec() {
        // ARRANGE / ACT / ASSERT
        assert!(load_file_entries(&[]).unwrap().is_empty());
    }

    #[test]
    fn load_file_entries_reads_file_correctly() {
        // ARRANGE
        let mut tmp = NamedTempFile::new().unwrap();
        tmp.write_all(b"hello").unwrap();
        let spec = format!("{}:dest/file.txt", tmp.path().to_str().unwrap());

        // ACT
        let entries = load_file_entries(&[spec]).unwrap();

        // ASSERT
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].path, "dest/file.txt");
        assert_eq!(entries[0].data, b"hello");
    }

    #[test]
    fn load_file_entries_missing_file_returns_error() {
        // ARRANGE / ACT / ASSERT
        assert!(load_file_entries(&["/nonexistent/file.bin:dst".to_owned()]).is_err());
    }
}
