//! Command-line arguments for the mumi rootfs builder.

use std::path::PathBuf;

use anyhow::{Context as _, Result};
use clap::Parser;
use erofs::FileContexts;

/// Rootfs image build arguments.
#[derive(Debug, Parser)]
#[command(name = env!("CARGO_PKG_NAME"))]
#[command(about = env!("CARGO_PKG_DESCRIPTION"))]
#[command(version = env!("CARGO_PKG_VERSION"))]
pub struct Args {
    /// Path to the rootfs directory.
    #[arg(short, long)]
    pub dir: PathBuf,

    /// Output EROFS image path.
    #[arg(short, long)]
    pub output: PathBuf,

    /// Optional `SELinux` `file_contexts` file.
    #[arg(short, long)]
    pub file_contexts: Option<PathBuf>,

    /// Zstd compression level for the EROFS image.
    #[arg(long, default_value_t = erofs::DEFAULT_ZSTD_COMPRESSION_LEVEL)]
    pub compression_level: i32,
}

impl Args {
    /// Parses the optional `file_contexts` file into [`FileContexts`].
    ///
    /// # Errors
    ///
    /// Returns an error if the file cannot be opened or parsed.
    pub fn load_file_contexts(&self) -> Result<Option<FileContexts>> {
        match self.file_contexts.as_ref() {
            Some(path) => {
                let file = std::fs::File::open(path)
                    .with_context(|| format!("Failed to open file_contexts: {}", path.display()))?;
                Ok(Some(
                    FileContexts::from_reader(file).context("Failed to parse file_contexts")?,
                ))
            }
            None => Ok(None),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::*;

    #[test]
    fn parse_args_default_compression() {
        // ACT
        let args = Args::parse_from(["mumi", "--dir", "/tmp", "--output", "/tmp/img"]);

        // ASSERT
        assert_eq!(args.dir, Path::new("/tmp"));
        assert_eq!(args.output, Path::new("/tmp/img"));
        assert!(args.file_contexts.is_none());
        assert_eq!(
            args.compression_level,
            erofs::DEFAULT_ZSTD_COMPRESSION_LEVEL
        );
    }

    #[test]
    fn parse_args_custom_compression() {
        // ACT
        let args = Args::parse_from([
            "mumi",
            "--dir",
            "/root",
            "--output",
            "/out.erofs",
            "--compression-level",
            "9",
        ]);

        // ASSERT
        assert_eq!(args.compression_level, 9);
    }

    #[test]
    fn parse_args_with_file_contexts() {
        // ACT
        let args = Args::parse_from([
            "mumi",
            "--dir",
            "/tmp",
            "--output",
            "/tmp/img",
            "--file-contexts",
            "/contexts",
        ]);

        // ASSERT
        assert_eq!(args.file_contexts, Some(PathBuf::from("/contexts")));
    }

    #[test]
    fn load_file_contexts_none() {
        // ARRANGE
        let args = Args::parse_from(["mumi", "--dir", "/tmp", "--output", "/tmp/img"]);

        // ACT
        let result = args.load_file_contexts().unwrap();

        // ASSERT
        assert!(result.is_none());
    }

    #[test]
    fn load_file_contexts_valid() {
        // ARRANGE
        let dir = tempfile::tempdir().unwrap();
        let fc_path = dir.path().join("fc");
        std::fs::write(&fc_path, b"/.*    system_u:object_r:file_t:s0\n").unwrap();
        let args = Args::parse_from([
            "mumi",
            "--dir",
            dir.path().to_str().unwrap(),
            "--output",
            dir.path().join("out.erofs").to_str().unwrap(),
            "--file-contexts",
            fc_path.to_str().unwrap(),
        ]);

        // ACT
        let result = args.load_file_contexts().unwrap();

        // ASSERT
        result.unwrap();
    }

    #[test]
    fn load_file_contexts_missing_file() {
        // ARRANGE
        let dir = tempfile::tempdir().unwrap();
        let args = Args::parse_from([
            "mumi",
            "--dir",
            dir.path().to_str().unwrap(),
            "--output",
            dir.path().join("out.erofs").to_str().unwrap(),
            "--file-contexts",
            "/nonexistent/contexts",
        ]);

        // ACT / ASSERT
        args.load_file_contexts().unwrap_err();
    }

    #[test]
    fn parse_args_short_flags() {
        // ACT
        let args = Args::parse_from(["mumi", "-d", "/src", "-o", "/dst.erofs"]);

        // ASSERT
        assert_eq!(args.dir, Path::new("/src"));
        assert_eq!(args.output, Path::new("/dst.erofs"));
    }
}
