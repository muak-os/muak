//! Command-line interface for ramune.

use core::str::FromStr;
use std::collections::HashMap;
use std::ffi::OsString;
use std::fs::File;
use std::io::Read as _;
use std::path::{Path, PathBuf};

use anyhow::{Context as _, Result, bail};
use clap::Parser;

use crate::archive;
use crate::error::RamuneError;

#[derive(Debug, Parser)]
#[command(name = env!("CARGO_PKG_NAME"))]
#[command(about = env!("CARGO_PKG_DESCRIPTION"))]
#[command(version = env!("CARGO_PKG_VERSION"))]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Parser)]
enum Command {
    Create {
        #[arg(short = 'f', long, value_name = "NAME=PATH:MODE", required = true)]
        file: Vec<FileEntry>,

        #[arg(short, long)]
        output: PathBuf,

        #[arg(long, default_value_t = crate::DEFAULT_ZSTD_COMPRESSION_LEVEL)]
        compression_level: i32,
    },
}

#[derive(Debug, Clone)]
struct FileEntry {
    name: String,
    path: PathBuf,
    mode: u32,
}

impl FromStr for FileEntry {
    type Err = anyhow::Error;

    fn from_str(input: &str) -> Result<Self> {
        let (name, rest) = input.split_once('=').ok_or_else(|| {
            anyhow::anyhow!("Invalid file format: {input:?}, expected NAME=PATH:MODE")
        })?;

        if name.is_empty() {
            bail!("File name must not be empty in: {input:?}");
        }

        let (path_str, mode_str) = rest.rsplit_once(':').ok_or_else(|| {
            anyhow::anyhow!("Invalid file format: {input:?}, expected NAME=PATH:MODE")
        })?;

        if path_str.is_empty() {
            bail!("File path must not be empty in: {input:?}");
        }

        let raw_mode = u32::from_str_radix(mode_str, 8).map_err(|_err| {
            anyhow::anyhow!("Invalid mode {mode_str:?} in {input:?}, expected octal")
        })?;

        let mode = if raw_mode & 0o170_000 == 0 {
            raw_mode | 0o100_000
        } else {
            raw_mode
        };

        Ok(FileEntry {
            name: name.to_owned(),
            path: PathBuf::from(path_str),
            mode,
        })
    }
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
            file,
            output,
            compression_level,
        } => run_create(&file, &output, compression_level),
    }
}

fn run_create(files: &[FileEntry], output: &Path, compression_level: i32) -> Result<()> {
    let entries = files;

    if entries.is_empty() {
        bail!("At least one --file is required");
    }

    let mut readers: HashMap<String, File> = HashMap::new();
    let mut archive_entries = Vec::with_capacity(entries.len());

    for entry in entries {
        let file = File::open(&entry.path).with_context(|| {
            format!(
                "Failed to open '{}' at {}",
                entry.name,
                entry.path.display()
            )
        })?;
        let len = file
            .metadata()
            .with_context(|| format!("Failed to get metadata for '{}'", entry.name))?
            .len();

        readers.insert(entry.name.clone(), file);
        archive_entries.push(crate::Entry {
            path: entry.name.clone(),
            mode: entry.mode,
            len,
        });
    }

    let mut output_file = File::create(output)
        .with_context(|| format!("Failed to create output file: {}", output.display()))?;

    archive::compressed(
        &mut archive_entries,
        &mut output_file,
        compression_level,
        |entry, w| {
            let reader = readers.get_mut(&entry.path).ok_or_else(|| {
                RamuneError::CpioError(format!("missing reader for '{}'", entry.path))
            })?;

            let mut limited = reader.take(entry.len);
            std::io::copy(&mut limited, w).map_err(|source| RamuneError::WriteError {
                file: entry.path.clone(),
                source,
            })?;

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn from_str_full() {
        // ACT
        let entry: FileEntry = "init=/sbin/init:755".parse().unwrap();

        // ASSERT
        assert_eq!(entry.name, "init");
        assert_eq!(entry.path, Path::new("/sbin/init"));
        assert_eq!(entry.mode, 0o100_755);
    }

    #[test]
    fn from_str_missing_equals() {
        // ACT / ASSERT
        assert!("foo".parse::<FileEntry>().is_err());
    }

    #[test]
    fn from_str_empty_name() {
        // ACT / ASSERT
        assert!("=/path:644".parse::<FileEntry>().is_err());
    }

    #[test]
    fn from_str_empty_path() {
        // ACT / ASSERT
        assert!("name=:644".parse::<FileEntry>().is_err());
    }

    #[test]
    fn from_str_missing_mode() {
        // ACT / ASSERT
        assert!("name=/path".parse::<FileEntry>().is_err());
    }

    #[test]
    fn from_str_invalid_mode() {
        // ACT / ASSERT
        assert!("name=/path:abc".parse::<FileEntry>().is_err());
    }

    #[test]
    fn from_str_path_with_colons() {
        // ACT
        let entry: FileEntry = "a=/some:path/file:644".parse().unwrap();

        // ASSERT
        assert_eq!(entry.name, "a");
        assert_eq!(entry.path, Path::new("/some:path/file"));
        assert_eq!(entry.mode, 0o100_644);
    }
}
