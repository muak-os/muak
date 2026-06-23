//! Command-line interface for ramune.

use std::ffi::OsString;
use std::io::Cursor;
use std::path::{Path, PathBuf};

use anyhow::{Context as _, Result};
use clap::{Parser, Subcommand};

use crate::Entry;
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
    Tail {
        #[arg(
            short = 'e',
            long = "entry",
            value_name = "SRC:DEST",
            value_parser = parse_extra
        )]
        entry: Vec<(PathBuf, PathBuf)>,

        #[arg(short, long)]
        output: PathBuf,

        #[arg(long, default_value_t = crate::DEFAULT_ZSTD_COMPRESSION_LEVEL)]
        compression_level: i32,
    },
}

fn parse_extra(raw: &str) -> core::result::Result<(PathBuf, PathBuf), String> {
    let invalid = || format!("expected SRC:DEST, got: {raw}");
    let mut parts = raw.split(':');
    let src = parts.next().ok_or_else(invalid)?;
    let dest = parts.next().ok_or_else(invalid)?;

    if src.is_empty() {
        return Err("source path must not be empty".to_owned());
    }
    if dest.is_empty() {
        return Err("destination name must not be empty".to_owned());
    }
    if parts.next().is_some() {
        return Err(invalid());
    }

    Ok((PathBuf::from(src), PathBuf::from(dest)))
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
        Command::Tail {
            entry,
            output,
            compression_level,
        } => run_tail(&entry, &output, compression_level),
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
            let file = std::fs::File::open(path)
                .context(format!("Failed to open file_contexts: {}", path.display()))?;
            Some(erofs::FileContexts::from_reader(file).context("Failed to parse file_contexts")?)
        }
        None => None,
    };

    let init_bytes = std::fs::read(init)
        .with_context(|| format!("Failed to read init binary: {}", init.display()))?;
    let rootfs_erofs =
        rootfs::prepare(rootfs_dir, file_contexts.as_ref(), rootfs_compression_level)
            .context("Failed to prepare rootfs")?;

    let init_len = init_bytes.len().try_into().unwrap_or(u64::MAX);
    let erofs_len = rootfs_erofs.len().try_into().unwrap_or(u64::MAX);
    let mut init_reader = Cursor::new(init_bytes);
    let mut erofs_reader = Cursor::new(rootfs_erofs);
    let mut entries = [
        Entry::new(Path::new("init"), 0o100_755, &mut init_reader, init_len),
        Entry::new(
            Path::new("rootfs.erofs"),
            0o100_644,
            &mut erofs_reader,
            erofs_len,
        ),
    ];

    let mut file = std::fs::File::create(output)
        .with_context(|| format!("Failed to create output file: {}", output.display()))?;
    crate::archive(&mut entries, compression_level, &mut file)
        .context("Failed to create initramfs")?;

    println!(
        "Successfully created initramfs at {} ({} bytes)",
        output.display(),
        initramfs_size(output)?
    );

    Ok(())
}

fn run_tail(entry: &[(PathBuf, PathBuf)], output: &Path, compression_level: i32) -> Result<()> {
    let sources: Vec<Vec<u8>> = entry
        .iter()
        .map(|entry| read_entry_source(&entry.0))
        .collect::<Result<_>>()?;
    let mut readers: Vec<Cursor<Vec<u8>>> = sources.into_iter().map(Cursor::new).collect();
    let mut entries: Vec<Entry<'_>> = entry
        .iter()
        .zip(readers.iter_mut())
        .map(|(entry, reader)| entry_from_source(&entry.0, entry.1.as_path(), reader))
        .collect::<Result<_>>()?;

    let mut file = std::fs::File::create(output)
        .with_context(|| format!("Failed to create output file: {}", output.display()))?;
    crate::archive(&mut entries, compression_level, &mut file)
        .context("Failed to build initramfs tail")?;

    println!(
        "Successfully created initramfs tail at {} ({} bytes)",
        output.display(),
        initramfs_size(output)?
    );

    Ok(())
}

fn read_entry_source(source: &Path) -> Result<Vec<u8>> {
    std::fs::read(source)
        .with_context(|| format!("Failed to read input entry: {}", source.display()))
}

fn entry_from_source<'a>(
    source: &Path,
    archive_path: &'a Path,
    reader: &'a mut Cursor<Vec<u8>>,
) -> Result<Entry<'a>> {
    let metadata = std::fs::metadata(source)
        .with_context(|| format!("Failed to inspect entry metadata: {}", source.display()))?;
    let readonly = metadata.permissions().readonly();
    let mode = if readonly { 0o100_444 } else { 0o100_644 };
    let len = reader.get_ref().len().try_into().unwrap_or(u64::MAX);

    Ok(Entry::new(archive_path, mode, reader, len))
}

fn initramfs_size(output: &Path) -> Result<u64> {
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
        let (path, name) = parse_extra(input).expect("parse");

        // ASSERT
        assert_eq!(path, PathBuf::from("/tmp/profile.toml"));
        assert_eq!(name, PathBuf::from("profile.toml"));
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
