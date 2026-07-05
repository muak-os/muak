//! Command-line interface for ramune.

use std::ffi::OsString;
use std::fs::File;
use std::io::Read as _;
use std::path::{Path, PathBuf};

use anyhow::{Context as _, Result};
use clap::{Parser, Subcommand};
use erofs::writer;

use crate::archive;
use crate::error::RamuneError;
use crate::rootfs;

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

/// Like [`run_from`] but returns an exit code (0 for success, 1 for error).
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

fn run_command(command: Command) -> Result<()> {
    match command {
        Command::Create {
            init,
            rootfs_dir,
            file_contexts,
            output,
            compression_level,
            rootfs_compression_level,
        } => run_create(
            &init,
            &rootfs_dir,
            file_contexts.as_deref(),
            &output,
            compression_level,
            rootfs_compression_level,
        ),
    }
}

fn run_create(
    init: &Path,
    rootfs_dir: &Path,
    file_contexts: Option<&Path>,
    output: &Path,
    compression_level: i32,
    rootfs_compression_level: i32,
) -> Result<()> {
    let file_contexts = match file_contexts {
        Some(path) => {
            let file = File::open(path)
                .context(format!("Failed to open file_contexts: {}", path.display()))?;
            Some(erofs::FileContexts::from_reader(file).context("Failed to parse file_contexts")?)
        }
        None => None,
    };

    let mut init_file = File::open(init)
        .with_context(|| format!("Failed to open init binary: {}", init.display()))?;
    let init_len = init_file
        .metadata()
        .with_context(|| format!("Failed to get metadata for init binary: {}", init.display()))?
        .len();

    let (erofs_plan, erofs_config, erofs_len) =
        rootfs::prepare_and_plan(rootfs_dir, file_contexts.as_ref(), rootfs_compression_level)
            .context("Failed to prepare rootfs")?;

    let mut entries = [
        crate::Entry {
            path: "init".into(),
            mode: 0o100_755,
            len: init_len,
        },
        crate::Entry {
            path: "rootfs.erofs".into(),
            mode: 0o100_644,
            len: erofs_len,
        },
    ];

    let mut output_file = File::create(output)
        .with_context(|| format!("Failed to create output file: {}", output.display()))?;

    archive::compressed(
        &mut entries,
        &mut output_file,
        compression_level,
        |entry, w| {
            match entry.path.as_str() {
                "init" => {
                    let mut limited = (&mut init_file).take(init_len);
                    std::io::copy(&mut limited, w).map_err(|source| RamuneError::WriteError {
                        file: String::new(),
                        source,
                    })?;
                }
                "rootfs.erofs" => writer::image(w, &erofs_plan, &erofs_config)
                    .map_err(|error| RamuneError::ErofsError(error.to_string()))?,
                other => return Err(RamuneError::CpioError(format!("unexpected entry: {other}"))),
            }
            Ok(())
        },
    )
    .context("Failed to create initramfs")?;

    let size = std::fs::metadata(output)
        .with_context(|| format!("Failed to read initramfs metadata: {}", output.display()))?
        .len();

    println!(
        "Successfully created initramfs at {} ({} bytes)",
        output.display(),
        size
    );

    Ok(())
}
