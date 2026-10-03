//! Resolve an image reference to its manifest digest.

use anyhow::{Context as _, Result};
use koci::registry;

/// Arguments of the `digest` subcommand.
#[derive(clap::Args, Debug)]
pub struct Args {
    /// Registry repository of the image.
    #[arg(short, long)]
    image: String,

    /// Tag to resolve.
    #[arg(short, long = "tag")]
    tag: String,
}

/// Execute the `digest` subcommand.
pub(crate) fn run(args: Args) -> Result<()> {
    let Args { image, tag } = args;

    let digest =
        registry::manifest_digest(&format!("{image}:{tag}")).context("Failed to resolve digest")?;
    println!("{digest}");

    Ok(())
}

#[cfg(test)]
mod tests {
    use clap::Parser as _;

    use crate::cli::{Args as CliArgs, Command};

    #[test]
    fn digest_subcommand_parses_image_and_tag() {
        // ARRANGE / ACT
        let args = CliArgs::try_parse_from([
            "koci",
            "digest",
            "--image",
            "ghcr.io/muak-os/toolchain",
            "--tag",
            "v1.0.0-beta",
        ])
        .expect("parse digest args");

        // ASSERT
        let Command::Digest(digest) = args.command else {
            panic!("expected digest command");
        };
        assert_eq!(digest.image, "ghcr.io/muak-os/toolchain");
        assert_eq!(digest.tag, "v1.0.0-beta");
    }
}
