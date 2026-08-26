use std::io::Write;
use std::path::Path;

use anyhow::{Context as _, Result, bail};
use clap::Parser as _;
use erofs::source;
use mumi::rootfs;

pub mod cli;

fn main() -> Result<()> {
    run(&cli::Args::parse())
}

fn run(args: &cli::Args) -> Result<()> {
    if !args.dir.is_dir() {
        bail!("Source directory does not exist: {}", args.dir.display());
    }

    let file_contexts = args.load_file_contexts()?;
    let mut output = std::fs::File::create(&args.output)
        .with_context(|| format!("Failed to create output file: {}", args.output.display()))?;
    build_image(
        &args.dir,
        file_contexts.as_ref(),
        args.compression_level,
        &mut output,
    )?;

    println!(
        "Created rootfs image at {} ({} bytes)",
        args.output.display(),
        output.metadata()?.len(),
    );

    Ok(())
}

fn build_image<W: Write>(
    dir: &Path,
    file_contexts: Option<&erofs::FileContexts>,
    compression_level: i32,
    writer: &mut W,
) -> Result<()> {
    rootfs::inject_required_dirs(dir)?;
    rootfs::ensure_default_resolv_conf(&dir.join("etc/resolv.conf"))?;

    let entries = source::collect_entries(dir).context("Failed to collect rootfs entries")?;
    let mumi_entries: Vec<mumi::image::Entry> = entries
        .iter()
        .map(|entry| mumi::image::Entry {
            path: entry.rel_path.clone(),
            size: entry.size,
            mode: entry.mode,
            symlink_target: entry.symlink_target.clone(),
        })
        .collect();
    let config = mumi::image::BuildConfig {
        compression_level,
        file_contexts: file_contexts.cloned(),
    };
    let mut measure_readers = rootfs::build_readers(dir, &entries)?;
    let mut measure_views = measure_readers.views();
    let image = mumi::image::build(&mumi_entries, &mut measure_views, &config)?;

    let mut write_readers = rootfs::build_readers(dir, &entries)?;
    let mut write_views = write_readers.views();
    image
        .write(writer, &mut write_views)
        .context("Failed to create EROFS image")?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use tempfile::NamedTempFile;

    use super::*;

    fn run_mumi(dir: &Path, fc: Option<&Path>, clevel: i32) -> Vec<u8> {
        let file_contexts = fc.map(|path| {
            erofs::FileContexts::from_reader(std::fs::File::open(path).unwrap())
                .expect("parse file_contexts")
        });
        let mut buf = Vec::new();
        build_image(dir, file_contexts.as_ref(), clevel, &mut buf).expect("build image");

        buf
    }

    #[test]
    fn empty_directory() {
        // ARRANGE
        let dir = tempfile::tempdir().unwrap();

        // ACT
        let image = run_mumi(dir.path(), None, 3);

        // ASSERT
        assert!(!image.is_empty());
        assert!(image.len().is_multiple_of(4096));
    }

    #[test]
    fn with_file() {
        // ARRANGE
        let dir = tempfile::tempdir().unwrap();
        let mut file = NamedTempFile::new_in(dir.path()).unwrap();
        file.write_all(b"hello world").unwrap();

        // ACT
        let image = run_mumi(dir.path(), None, 3);

        // ASSERT
        assert!(!image.is_empty());
    }

    #[test]
    fn missing_source_dir_errors() {
        // ARRANGE
        let dir = tempfile::tempdir().unwrap();
        let nonexistent = dir.path().join("does-not-exist");
        let output = dir.path().join("out.erofs");
        let args = cli::Args::parse_from([
            "mumi",
            "--dir",
            nonexistent.to_str().unwrap(),
            "--output",
            output.to_str().unwrap(),
        ]);

        // ACT / ASSERT
        assert!(run(&args).is_err());
    }

    #[test]
    fn invalid_compression_level_errors() {
        // ARRANGE
        let dir = tempfile::tempdir().unwrap();
        let output = dir.path().join("out.erofs");
        let args = cli::Args::parse_from([
            "mumi",
            "--dir",
            dir.path().to_str().unwrap(),
            "--output",
            output.to_str().unwrap(),
            "--compression-level",
            "999999",
        ]);

        // ACT / ASSERT
        assert!(run(&args).is_err());
    }

    #[test]
    fn missing_file_contexts_errors() {
        // ARRANGE
        let dir = tempfile::tempdir().unwrap();
        let output = dir.path().join("out.erofs");
        let args = cli::Args::parse_from([
            "mumi",
            "--dir",
            dir.path().to_str().unwrap(),
            "--output",
            output.to_str().unwrap(),
            "--file-contexts",
            "/nonexistent/file_contexts",
        ]);

        // ACT / ASSERT
        assert!(run(&args).is_err());
    }

    #[test]
    fn injects_required_dirs() {
        // ARRANGE
        let dir = tempfile::tempdir().unwrap();

        // ACT
        rootfs::inject_required_dirs(dir.path()).unwrap();

        // ASSERT
        for dir_name in rootfs::REQUIRED_DIRS {
            assert!(
                dir.path().join(dir_name).exists(),
                "missing required dir: {dir_name}",
            );
        }
    }

    #[test]
    fn injects_resolv_conf_when_missing() {
        // ARRANGE
        let dir = tempfile::tempdir().unwrap();
        let resolv = dir.path().join("etc/resolv.conf");

        // ACT
        rootfs::ensure_default_resolv_conf(&resolv).unwrap();

        // ASSERT
        resolv.symlink_metadata().unwrap();
        let target = std::fs::read_link(&resolv).unwrap();
        assert_eq!(target, std::path::Path::new("/run/resolv.conf"));
    }

    #[test]
    fn reproducible_output() {
        // ARRANGE
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("data"), b"content").unwrap();

        // ACT
        let img1 = run_mumi(dir.path(), None, 3);
        let img2 = run_mumi(dir.path(), None, 3);

        // ASSERT
        assert_eq!(img1, img2);
    }

    #[test]
    fn with_file_contexts() {
        // ARRANGE
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("f"), b"x").unwrap();
        let fc_path = dir.path().join("file_contexts");
        std::fs::write(&fc_path, "/.*    system_u:object_r:file_t:s0\n").unwrap();

        // ACT
        let image = run_mumi(dir.path(), Some(&fc_path), 3);

        // ASSERT
        assert!(!image.is_empty());
        assert!(image.len().is_multiple_of(4096));
    }

    #[test]
    fn run_creates_output_file() {
        // ARRANGE
        let dir = tempfile::tempdir().unwrap();
        let output = dir.path().join("out.erofs");
        let args = cli::Args::parse_from([
            "mumi",
            "--dir",
            dir.path().to_str().unwrap(),
            "--output",
            output.to_str().unwrap(),
        ]);

        // ACT
        run(&args).unwrap();

        // ASSERT
        assert!(output.exists());
        assert!(output.metadata().unwrap().len() > 0);
    }

    #[test]
    fn run_with_file_contexts_from_args() {
        // ARRANGE
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("f"), b"x").unwrap();
        let fc_path = dir.path().join("fc");
        std::fs::write(&fc_path, "/.*    system_u:object_r:file_t:s0\n").unwrap();
        let output = dir.path().join("out.erofs");
        let args = cli::Args::parse_from([
            "mumi",
            "--dir",
            dir.path().to_str().unwrap(),
            "--output",
            output.to_str().unwrap(),
            "--file-contexts",
            fc_path.to_str().unwrap(),
        ]);

        // ACT
        run(&args).unwrap();

        // ASSERT
        assert!(output.exists());
    }
}
