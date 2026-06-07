//! Command-line interface for ramune.

use std::ffi::OsString;
use std::path::PathBuf;

use anyhow::{Context as _, Result};
use clap::{Parser, Subcommand};

use crate::{CreateConfig, ExtendConfig, ExtraFile};

#[derive(Debug, Parser)]
#[command(name = env!("CARGO_PKG_NAME"))]
#[command(about = env!("CARGO_PKG_DESCRIPTION"))]
#[command(version = env!("CARGO_PKG_VERSION"))]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    Create {
        #[arg(short, long)]
        init: PathBuf,

        #[arg(short, long)]
        rootfs_dir: PathBuf,

        #[arg(short, long)]
        file_contexts: Option<PathBuf>,

        #[arg(short, long)]
        output: PathBuf,

        #[arg(long, default_value_t = crate::DEFAULT_ZSTD_COMPRESSION_LEVEL)]
        compression_level: i32,

        #[arg(long, default_value_t = ::erofs::DEFAULT_ZSTD_COMPRESSION_LEVEL)]
        rootfs_compression_level: i32,
    },
    Extend {
        #[arg(short, long)]
        base: PathBuf,

        #[arg(
            short = 'e',
            long = "extra",
            value_name = "SRC:DEST[:COMPRESS]",
            value_parser = parse_extra
        )]
        extra: Vec<(PathBuf, String, bool)>,

        #[arg(short, long)]
        output: PathBuf,

        #[arg(long, default_value_t = crate::DEFAULT_ZSTD_COMPRESSION_LEVEL)]
        compression_level: i32,
    },
}

fn parse_extra(raw: &str) -> core::result::Result<(PathBuf, String, bool), String> {
    let mut parts = raw.split(':');
    let src = parts.next().ok_or("missing source path")?;
    let dest = parts
        .next()
        .ok_or_else(|| format!("expected SRC:DEST[:COMPRESS], got: {raw}"))?;
    let compress = match parts.next() {
        None | Some("") => false,
        Some(flag) => matches!(flag, "true" | "1" | "yes" | "compress"),
    };

    if src.is_empty() {
        return Err("source path must not be empty".to_owned());
    }
    if dest.is_empty() {
        return Err("destination name must not be empty".to_owned());
    }

    Ok((PathBuf::from(src), dest.to_owned(), compress))
}

/// Runs the CLI from a caller-provided argument iterator.
///
/// # Errors
///
/// Returns an error when argument parsing fails or when the requested command fails.
pub fn run_from<I, T>(args: I) -> Result<()>
where
    I: IntoIterator<Item = T>,
    T: Into<OsString> + Clone,
{
    let args = Cli::parse_from(args);
    run_command(args.command)
}

/// Like `run_from` but returns an exit code (0 for success, 1 for error).
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

/// Runs the CLI from the process's `std::env::args_os`.
#[must_use]
pub fn run() -> i32 {
    run_with(std::env::args_os())
}

fn run_command(command: Command) -> Result<()> {
    match command {
        Command::Create {
            init,
            rootfs_dir,
            file_contexts,
            output,
            compression_level,
            rootfs_compression_level,
        } => {
            let file_contexts = match file_contexts.as_ref() {
                Some(path) => {
                    let file = std::fs::File::open(path)
                        .context(format!("Failed to open file_contexts: {}", path.display()))?;
                    Some(
                        erofs::FileContexts::from_reader(file)
                            .context("Failed to parse file_contexts")?,
                    )
                }
                None => None,
            };

            let config = CreateConfig {
                init: &init,
                rootfs_dir: &rootfs_dir,
                file_contexts: file_contexts.as_ref(),
                compression_level,
                rootfs_compression_level,
            };

            let mut file = std::fs::File::create(&output)
                .with_context(|| format!("Failed to create output file: {}", output.display()))?;
            crate::create(&config, &mut file).context("Failed to create initramfs")?;
            let size = initramfs_size(&output)?;

            println!(
                "Successfully created initramfs at {} ({} bytes)",
                output.display(),
                size
            );

            Ok(())
        }
        Command::Extend {
            base,
            extra,
            output,
            compression_level,
        } => {
            let entries: Vec<ExtraFile<'_>> = extra
                .iter()
                .map(|entry| ExtraFile {
                    name: entry.1.clone(),
                    path: &entry.0,
                    compress: entry.2,
                })
                .collect();

            let config = ExtendConfig {
                base: &base,
                extra_files: &entries,
                compression_level,
            };

            let mut file = std::fs::File::create(&output)
                .with_context(|| format!("Failed to create output file: {}", output.display()))?;
            crate::extend(&config, &mut file).context("Failed to build initramfs")?;

            println!("Successfully created initramfs at {}", output.display());

            Ok(())
        }
    }
}

fn initramfs_size(output: &std::path::Path) -> Result<u64> {
    Ok(std::fs::metadata(output)
        .with_context(|| format!("Failed to read initramfs metadata: {}", output.display()))?
        .len())
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::*;

    #[test]
    fn initramfs_size_missing_output_reports_path() {
        // ARRANGE
        let path = Path::new("/nonexistent/initramfs.img");

        // ACT
        let error = initramfs_size(path).expect_err("missing file should error");

        // ASSERT
        assert!(
            error
                .to_string()
                .contains("Failed to read initramfs metadata: /nonexistent/initramfs.img")
        );
    }

    #[test]
    fn parse_extra_plain_file() {
        // ARRANGE
        let input = "/tmp/profile.toml:profile.toml";

        // ACT
        let (path, name, compress) = parse_extra(input).expect("parse");

        // ASSERT
        assert_eq!(path, PathBuf::from("/tmp/profile.toml"));
        assert_eq!(name, "profile.toml");
        assert!(!compress);
    }

    #[test]
    fn parse_extra_compress() {
        // ARRANGE
        let input = "/tmp/ext-dir:extensions/qemu.erofs:true";

        // ACT
        let (path, name, compress) = parse_extra(input).expect("parse");

        // ASSERT
        assert_eq!(path, PathBuf::from("/tmp/ext-dir"));
        assert_eq!(name, "extensions/qemu.erofs");
        assert!(compress);
    }

    #[test]
    fn parse_extra_compress_with_1() {
        // ARRANGE
        let input = "/tmp/dir:ext.erofs:1";

        // ACT
        let (_, _, compress) = parse_extra(input).expect("parse");

        // ASSERT
        assert!(compress);
    }

    #[test]
    fn parse_extra_no_compress_flag() {
        // ARRANGE
        let input = "/tmp/data:data.txt";

        // ACT
        let (_, _, compress) = parse_extra(input).expect("parse");

        // ASSERT
        assert!(!compress);
    }

    #[test]
    fn parse_extra_missing_dest() {
        // ARRANGE
        let input = "/tmp/file";

        // ACT
        let err = parse_extra(input).expect_err("should fail");

        // ASSERT
        assert!(err.contains("expected SRC:DEST"));
    }

    #[test]
    fn parse_extra_empty_src() {
        // ARRANGE
        let input = ":dest";

        // ACT
        let err = parse_extra(input).expect_err("should fail");

        // ASSERT
        assert!(err.contains("source path must not be empty"));
    }

    #[test]
    fn parse_extra_empty_dest() {
        // ARRANGE
        let input = "/tmp/src:";

        // ACT
        let err = parse_extra(input).expect_err("should fail");

        // ASSERT
        assert!(err.contains("destination name must not be empty"));
    }
}
