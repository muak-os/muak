//! Push files as a scratch OCI image.

use anyhow::{Context as _, Result};
use koci::{error, push};
use oci::arch;
use oci::arch::Arch;

use super::parse_arch;

/// Arguments of the `push` subcommand.
#[derive(clap::Args, Debug)]
pub struct Args {
    /// Image reference to push (e.g. `ghcr.io/org/catalog-core:v1`).
    #[arg(short, long)]
    image: String,

    /// Additional tag(s) for the pushed manifest.
    #[arg(short, long = "tag")]
    tags: Vec<String>,

    /// Architecture recorded in the image config (default: host).
    #[arg(short, long, value_parser = parse_arch)]
    arch: Option<Arch>,

    /// File(s) to pack into the image, `PATH[:ARCHIVE_PATH]`.
    #[arg(long = "file", value_name = "PATH[:NAME]", required = true)]
    files: Vec<String>,
}

/// Execute the `push` subcommand.
pub(crate) fn run(args: Args) -> Result<()> {
    let Args {
        image,
        tags,
        arch,
        files,
    } = args;

    let entries = files
        .iter()
        .map(|spec| push::parse_entry(spec))
        .collect::<error::Result<Vec<_>>>()
        .context("Failed to parse files")?;
    let target_arch = arch.unwrap_or(arch::host());

    let pushed =
        push::files(&image, &tags, &target_arch, &entries).context("Failed to push image")?;
    println!("Successfully pushed {image} (manifest {})", pushed.digest);

    Ok(())
}

#[cfg(test)]
mod tests {
    use clap::Parser as _;

    use crate::cli::{Args as CliArgs, Command};

    #[test]
    fn push_subcommand_parses_image_tags_arch_and_files() {
        // ARRANGE
        let args = CliArgs::try_parse_from([
            "koci",
            "push",
            "--image",
            "repo:test",
            "--tag",
            "v1",
            "--arch",
            "arm64",
            "--file",
            "catalog.toml",
            "--file",
            "extra.bin:data/extra.bin",
        ])
        .expect("parse push args");

        // ACT
        let Command::Push(pushed) = args.command else {
            panic!("expected push command");
        };

        // ASSERT
        assert_eq!(pushed.image, "repo:test");
        assert_eq!(pushed.tags, vec!["v1".to_owned()]);
        assert!(matches!(pushed.arch, Some(oci::arch::Arch::Arm64)));
        assert_eq!(
            pushed.files,
            vec![
                "catalog.toml".to_owned(),
                "extra.bin:data/extra.bin".to_owned()
            ]
        );
    }

    #[test]
    fn push_requires_at_least_one_file() {
        // ARRANGE / ACT
        let error = CliArgs::try_parse_from(["koci", "push", "--image", "repo:test"])
            .expect_err("push without files should not parse");

        // ASSERT
        assert!(error.to_string().contains("--file"));
    }
}
