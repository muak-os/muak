//! Repack a published image's layers into eStargz, in place.

use anyhow::{Context as _, Result};
use koci::repack;
use tui::style;

/// Arguments of the `repack` subcommand.
#[derive(clap::Args, Debug)]
pub struct Args {
    /// Image reference with a tag, such as `ghcr.io/org/image:tag`.
    #[arg(short, long)]
    image: String,

    /// Additional tag(s) to publish the repacked manifest under.
    #[arg(short, long = "tag")]
    tags: Vec<String>,
}

/// Execute the `repack` subcommand.
pub(crate) fn run(args: Args) -> Result<()> {
    let Args { image, tags } = args;
    let digest = repack::run(&image, &tags).context("Failed to repack image")?;
    let message = format!("Successfully repacked {image} (manifest {digest})");
    println!("{}", style::success(&message));

    Ok(())
}

#[cfg(test)]
mod tests {
    use clap::Parser as _;

    use crate::cli::{Args as CliArgs, Command};

    #[test]
    fn repack_subcommand_parses_image_and_tags() {
        // ARRANGE
        let args =
            CliArgs::try_parse_from(["koci", "repack", "--image", "repo:test", "--tag", "latest"])
                .expect("parse repack args");

        // ACT
        let Command::Repack(repacked) = args.command else {
            panic!("expected repack command");
        };

        // ASSERT
        assert_eq!(repacked.image, "repo:test");
        assert_eq!(repacked.tags, vec!["latest".to_owned()]);
    }
}
