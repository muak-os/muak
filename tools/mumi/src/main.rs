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

    rootfs::inject_required_dirs(&args.dir)?;
    rootfs::ensure_default_resolv_conf(&args.dir.join("etc/resolv.conf"))?;

    let entries = source::collect_entries(&args.dir).context("Failed to collect rootfs entries")?;
    let mumi_entries: Vec<mumi::image::Entry> = entries
        .iter()
        .map(|entry| mumi::image::Entry {
            path: entry.rel_path.clone(),
            size: entry.size,
            mode: entry.mode,
            symlink_target: entry.symlink_target.clone(),
        })
        .collect();
    let mut readers = rootfs::build_readers(&args.dir, &entries)?;

    let config = mumi::image::BuildConfig {
        compression_level: args.compression_level,
        file_contexts,
    };
    let image = mumi::image::build("rootfs", &mumi_entries, &mut readers, &config)?;

    let mut output = std::fs::File::create(&args.output)
        .with_context(|| format!("Failed to create output file: {}", args.output.display()))?;
    image
        .write(&mut output)
        .context("Failed to create EROFS image")?;

    println!(
        "Created rootfs image at {} ({} bytes)",
        args.output.display(),
        image.len()
    );

    Ok(())
}

#[cfg(test)]
mod tests {
    use std::io::Write as _;
    use std::path::Path;

    use erofs::source;
    use tempfile::NamedTempFile;

    use super::*;

    fn run_mumi(dir: &Path, fc: Option<&Path>, clevel: i32) -> Vec<u8> {
        rootfs::inject_required_dirs(dir).unwrap();
        rootfs::ensure_default_resolv_conf(&dir.join("etc/resolv.conf")).unwrap();

        let entries = source::collect_entries(dir).unwrap();
        let mumi_entries: Vec<mumi::image::Entry> = entries
            .iter()
            .map(|entry| mumi::image::Entry {
                path: entry.rel_path.clone(),
                size: entry.size,
                mode: entry.mode,
                symlink_target: entry.symlink_target.clone(),
            })
            .collect();
        let mut readers = rootfs::build_readers(dir, &entries).unwrap();

        let fc_parsed = fc.and_then(|fc_path| {
            let fc_file = std::fs::File::open(fc_path).ok()?;
            erofs::FileContexts::from_reader(fc_file).ok()
        });
        let config = mumi::image::BuildConfig {
            compression_level: clevel,
            file_contexts: fc_parsed,
        };
        let image = mumi::image::build("rootfs", &mumi_entries, &mut readers, &config).unwrap();

        let mut buf = Vec::new();
        image.write(&mut buf).unwrap();
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
