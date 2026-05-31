use std::ffi::OsString;
use std::fs::File;
use std::path::PathBuf;

use anyhow::{Context as _, Result, bail, ensure};
use clap::{Parser, Subcommand};
use esp::{Arch, EspFile, EspSpec, EspSpecBuilder};

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

    Raw {
        #[arg(short, long, help = "Path to the UKI .efi file")]
        uki: PathBuf,

        #[arg(short, long, help = "Path for the output .raw file")]
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

        #[arg(long, help = "Compress the output with zstd at the given level")]
        compression_level: Option<i32>,
    },
}

/// Parses the architecture string into an `esp::Arch` value.
fn parse_arch(arch: &str) -> Result<Arch> {
    match arch {
        "x86_64" => Ok(Arch::X86_64),
        "aarch64" => Ok(Arch::Aarch64),
        other => bail!("Unsupported architecture: '{other}'. Use x86_64 or aarch64."),
    }
}

/// Parses a `src:dst` file spec into `(src_path, dst_path)`.
fn parse_file_spec(spec: &str) -> Result<(&str, &str)> {
    let (src, dst) = spec
        .split_once(':')
        .context(format!("Invalid file spec '{spec}': expected src:dst"))?;
    ensure!(!src.is_empty(), "File source path is empty in '{spec}'");
    ensure!(
        !dst.is_empty(),
        "File destination path is empty in '{spec}'"
    );
    Ok((src, dst))
}

/// Loads extra file entries from `src:dst` specs.
fn load_file_entries(specs: &[String]) -> Result<Vec<EspFile>> {
    specs
        .iter()
        .map(|spec| {
            let (src, dst) = parse_file_spec(spec)?;
            let data = std::fs::read(src).context(format!("Failed to read file '{src}'"))?;
            Ok(EspFile {
                path: dst.to_owned(),
                data,
            })
        })
        .collect()
}

/// Builds an ESP spec from UKI bytes and extra file entries.
fn build_spec(arch: Arch, uki: Vec<u8>, extra_files: Vec<EspFile>) -> Result<EspSpec> {
    EspSpecBuilder::default()
        .with_uki(arch, uki)
        .context("Failed to add UKI to ESP spec")?
        .add_files(extra_files)
        .context("Failed to add extra files to ESP spec")?
        .build()
        .context("Failed to build ESP spec")
}

fn run_command(command: Command) -> Result<()> {
    match command {
        Command::Iso {
            uki,
            output,
            arch,
            files,
        } => {
            let arch = parse_arch(&arch)?;
            let uki_bytes =
                std::fs::read(&uki).context(format!("Failed to read {}", uki.display()))?;
            let extra_files = load_file_entries(&files)?;
            let spec = build_spec(arch, uki_bytes, extra_files)?;
            let mut file =
                File::create(&output).context(format!("Failed to create {}", output.display()))?;
            crate::build_iso(&spec, &mut file).context("Failed to build ISO")?;
            let size = file
                .metadata()
                .context(format!("Failed to stat {}", output.display()))?
                .len();
            println!("ISO written to {} ({} bytes)", output.display(), size);
            Ok(())
        }
        Command::Raw {
            uki,
            output,
            arch,
            files,
            compression_level,
        } => {
            let arch = parse_arch(&arch)?;
            let uki_bytes =
                std::fs::read(&uki).context(format!("Failed to read {}", uki.display()))?;
            let extra_files = load_file_entries(&files)?;
            let spec = build_spec(arch, uki_bytes, extra_files)?;
            let mut file =
                File::create(&output).context(format!("Failed to create {}", output.display()))?;
            crate::build_raw(&spec, &mut file, compression_level)
                .context("Failed to build disk image")?;
            let size = file
                .metadata()
                .context(format!("Failed to stat {}", output.display()))?
                .len();
            println!("Image written to {} ({} bytes)", output.display(), size);
            Ok(())
        }
    }
}

/// Runs the CLI from a caller-provided argument iterator.
///
/// # Errors
///
/// Returns an error if argument parsing fails or the requested image build fails.
pub fn run_from<I, T>(args: I) -> Result<()>
where
    I: IntoIterator<Item = T>,
    T: Into<OsString> + Clone,
{
    let args = Args::parse_from(args);
    run_command(args.command)
}

pub fn run_with<I, T>(args: I) -> i32
where
    I: IntoIterator<Item = T>,
    T: Into<OsString> + Clone,
{
    match run_from(args) {
        Ok(()) => 0,
        Err(error) => {
            eprintln!("Error: {error:?}");
            1
        }
    }
}

#[must_use]
pub fn run() -> i32 {
    run_with(std::env::args_os())
}

#[cfg(test)]
mod tests {
    use std::io::Write as _;

    use clap::CommandFactory as _;
    use clap::Parser as _;
    use esp::Arch;
    use tempfile::NamedTempFile;

    use super::{
        Args, Command, load_file_entries, parse_arch, parse_file_spec, run_from, run_with,
    };

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
        parse_arch("riscv64").unwrap_err();
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
        parse_file_spec("nodivider").unwrap_err();
    }

    #[test]
    fn parse_file_spec_empty_src_returns_error() {
        // ARRANGE / ACT / ASSERT
        parse_file_spec(":dst").unwrap_err();
    }

    #[test]
    fn parse_file_spec_empty_dst_returns_error() {
        // ARRANGE / ACT / ASSERT
        parse_file_spec("src:").unwrap_err();
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
        let entry = entries.first().expect("file entry must exist");
        assert_eq!(entry.path, "dest/file.txt");
        assert_eq!(entry.data, b"hello");
    }

    #[test]
    fn load_file_entries_missing_file_returns_error() {
        // ARRANGE / ACT / ASSERT
        load_file_entries(&["/nonexistent/file.bin:dst".to_owned()]).unwrap_err();
    }

    #[test]
    fn raw_subcommand_parses_compression_level() {
        // ARRANGE
        let args = Args::try_parse_from([
            "miso",
            "raw",
            "--uki",
            "input.efi",
            "--output",
            "output.raw.zst",
            "--compression-level",
            "3",
        ])
        .expect("parse args");

        // ACT
        let command = args.command;

        // ASSERT
        assert!(matches!(
            command,
            Command::Raw {
                uki,
                output,
                arch,
                files,
                compression_level,
            } if uki.as_path() == std::path::Path::new("input.efi")
                && output.as_path() == std::path::Path::new("output.raw.zst")
                && arch == "aarch64"
                && files.is_empty()
                && compression_level == Some(3)
        ));
    }

    #[test]
    fn raw_subcommand_defaults_to_aarch64_without_compression() {
        // ARRANGE
        let args = Args::try_parse_from([
            "miso",
            "raw",
            "--uki",
            "input.efi",
            "--output",
            "output.raw",
        ])
        .expect("parse args");

        // ACT
        let command = args.command;

        // ASSERT
        assert!(matches!(
            command,
            Command::Raw {
                arch,
                compression_level,
                ..
            } if arch == "aarch64" && compression_level.is_none()
        ));
    }

    #[test]
    fn iso_subcommand_defaults_to_x86_64() {
        // ARRANGE
        let args = Args::try_parse_from([
            "miso",
            "iso",
            "--uki",
            "input.efi",
            "--output",
            "output.iso",
        ])
        .expect("parse args");

        // ACT
        let command = args.command;

        // ASSERT
        assert!(matches!(
            command,
            Command::Iso {
                uki,
                output,
                arch,
                files,
            } if uki.as_path() == std::path::Path::new("input.efi")
                && output.as_path() == std::path::Path::new("output.iso")
                && arch == "x86_64"
                && files.is_empty()
        ));
    }

    #[test]
    fn clap_command_factory_registers_both_subcommands() {
        // ARRANGE / ACT
        let command = Args::command();
        let subcommands = command.get_subcommands().map(clap::Command::get_name);

        // ASSERT
        assert_eq!(subcommands.collect::<Vec<_>>(), vec!["iso", "raw"]);
    }

    #[test]
    fn run_from_builds_iso_successfully() {
        // ARRANGE
        let uki = NamedTempFile::new().expect("temp uki file");
        std::fs::write(uki.path(), b"MZtest-uki").expect("write uki bytes");
        let output = NamedTempFile::new().expect("temp output file");

        // ACT
        let result = run_from([
            "miso",
            "iso",
            "--uki",
            uki.path().to_str().expect("uki path must be valid utf-8"),
            "--output",
            output
                .path()
                .to_str()
                .expect("output path must be valid utf-8"),
        ]);

        // ASSERT
        result.expect("run_from iso must succeed");
        let bytes = std::fs::read(output.path()).expect("read iso output");
        assert!(!bytes.is_empty());
    }

    #[test]
    fn run_from_builds_raw_successfully() {
        // ARRANGE
        let uki = NamedTempFile::new().expect("temp uki file");
        std::fs::write(uki.path(), b"MZtest-raw-uki").expect("write uki bytes");
        let output = NamedTempFile::new().expect("temp output file");

        // ACT
        let result = run_from([
            "miso",
            "raw",
            "--uki",
            uki.path().to_str().expect("uki path must be valid utf-8"),
            "--output",
            output
                .path()
                .to_str()
                .expect("output path must be valid utf-8"),
        ]);

        // ASSERT
        result.expect("run_from raw must succeed");
        let bytes = std::fs::read(output.path()).expect("read raw output");
        assert!(!bytes.is_empty());
    }

    #[test]
    fn run_from_reports_missing_uki_for_iso() {
        // ARRANGE
        let output = NamedTempFile::new().expect("temp output file");

        // ACT
        let err = run_from([
            "miso",
            "iso",
            "--uki",
            "/nonexistent/uki.efi",
            "--output",
            output
                .path()
                .to_str()
                .expect("output path must be valid utf-8"),
        ])
        .expect_err("missing uki must fail");

        // ASSERT
        assert!(err.to_string().contains("Failed to read"));
    }

    #[test]
    fn run_from_reports_invalid_compression_level_for_raw() {
        // ARRANGE
        let uki = NamedTempFile::new().expect("temp uki file");
        std::fs::write(uki.path(), b"MZtest-raw-uki").expect("write uki bytes");
        let output = NamedTempFile::new().expect("temp output file");

        // ACT
        let err = run_from([
            "miso",
            "raw",
            "--uki",
            uki.path().to_str().expect("uki path must be valid utf-8"),
            "--output",
            output
                .path()
                .to_str()
                .expect("output path must be valid utf-8"),
            "--compression-level",
            "999999",
        ])
        .expect_err("invalid compression level must fail");

        // ASSERT
        assert!(err.to_string().contains("Failed to build disk image"));
    }

    #[test]
    fn run_with_returns_zero_for_success() {
        // ARRANGE
        let uki = NamedTempFile::new().expect("temp uki file");
        std::fs::write(uki.path(), b"MZtest-uki").expect("write uki bytes");
        let output = NamedTempFile::new().expect("temp output file");

        // ACT
        let exit_code = run_with([
            "miso",
            "iso",
            "--uki",
            uki.path().to_str().expect("uki path must be valid utf-8"),
            "--output",
            output
                .path()
                .to_str()
                .expect("output path must be valid utf-8"),
        ]);

        // ASSERT
        assert_eq!(exit_code, 0);
    }

    #[test]
    fn run_with_returns_one_for_error() {
        // ARRANGE
        let output = NamedTempFile::new().expect("temp output file");

        // ACT
        let exit_code = run_with([
            "miso",
            "iso",
            "--uki",
            "/nonexistent/uki.efi",
            "--output",
            output
                .path()
                .to_str()
                .expect("output path must be valid utf-8"),
        ]);

        // ASSERT
        assert_eq!(exit_code, 1);
    }
}
