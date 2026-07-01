//! Command-line interface for miso.

use std::ffi::OsString;
use std::fs::File;
use std::path::{Path, PathBuf};

use anyhow::{Context as _, Result, bail, ensure};
use clap::{Parser, Subcommand};
use esp::model::{Arch, EspFile, EspSpec};

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

/// Parses the architecture string into an `esp::model::Arch` value.
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

fn build_spec(uki: &Path, arch: Arch, files: &[String]) -> Result<EspSpec> {
    let uki_file = File::open(uki).context(format!("Failed to read {}", uki.display()))?;
    let uki_len = uki_file.metadata()?.len();
    let boot = EspFile::boot(arch, uki_file, uki_len);

    let mut all_files = Vec::with_capacity(files.len().saturating_add(1));
    all_files.push(boot);
    for spec in files {
        let (src, dst) = parse_file_spec(spec)?;
        let file = std::fs::File::open(src)?;
        let size = file.metadata()?.len();
        all_files.push(EspFile {
            path: dst.to_owned(),
            reader: Box::new(file),
            size,
        });
    }

    EspSpec::builder()
        .add_files(all_files)
        .context("Failed to add files to ESP spec")?
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
            let mut spec = build_spec(&uki, arch, &files)?;
            let mut writer =
                File::create(&output).context(format!("Failed to create {}", output.display()))?;
            crate::build_iso(&mut spec, &mut writer).context("Failed to build ISO")?;
            let size = writer
                .metadata()
                .context(format!("Failed to stat {}", output.display()))?
                .len();
            println!("ISO written to {} ({} bytes)", output.display(), size);
        }

        Command::Raw {
            uki,
            output,
            arch,
            files,
            compression_level,
        } => {
            let arch = parse_arch(&arch)?;
            let mut spec = build_spec(&uki, arch, &files)?;
            let mut writer =
                File::create(&output).context(format!("Failed to create {}", output.display()))?;
            crate::build_raw(&mut spec, &mut writer, compression_level)
                .context("Failed to build disk image")?;
            let size = writer
                .metadata()
                .context(format!("Failed to stat {}", output.display()))?
                .len();
            println!("Image written to {} ({} bytes)", output.display(), size);
        }
    }

    Ok(())
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

/// Runs the CLI with the given arguments and returns an exit code.
pub fn run_with<I, T>(args: I) -> i32
where
    I: IntoIterator<Item = T>,
    T: Into<OsString> + Clone,
{
    match run_from(args) {
        Ok(()) => 0,
        Err(err) => {
            eprintln!("{err:?}");
            1
        }
    }
}

/// Runs the CLI from process arguments.
#[must_use]
pub fn run() -> i32 {
    run_with(std::env::args_os())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_arch_recognises_known_values() {
        // ARRANGE / ACT / ASSERT
        assert_eq!(parse_arch("x86_64").expect("x86_64"), Arch::X86_64);
        assert_eq!(parse_arch("aarch64").expect("aarch64"), Arch::Aarch64);
    }

    #[test]
    fn parse_arch_rejects_unknown_values() {
        // ARRANGE / ACT
        let result = parse_arch("riscv64");

        // ASSERT
        result.unwrap_err();
    }

    #[test]
    fn parse_file_spec_splits_at_first_colon() {
        // ARRANGE / ACT
        let (src, dst) = parse_file_spec("src:dst/path").expect("split must succeed");

        // ASSERT
        assert_eq!(src, "src");
        assert_eq!(dst, "dst/path");
    }

    #[test]
    fn parse_file_spec_rejects_empty_source() {
        // ARRANGE / ACT
        let result = parse_file_spec(":dst");

        // ASSERT
        result.unwrap_err();
    }

    #[test]
    fn parse_file_spec_rejects_empty_destination() {
        // ARRANGE / ACT
        let result = parse_file_spec("src:");

        // ASSERT
        result.unwrap_err();
    }
}
