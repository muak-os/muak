//! Annotate an OCI image with file sizes.

use anyhow::{Context as _, Result};
use koci::annotations;

/// Arguments of the `annotate` subcommand.
#[derive(clap::Args, Debug)]
pub struct Args {
    #[arg(short, long)]
    image: String,

    #[arg(long, value_name = "KEY")]
    annotation: String,

    #[arg(long, value_name = "PREFIX")]
    exclude: Vec<String>,
}

/// Execute the `annotate` subcommand.
pub(crate) fn run(args: Args) -> Result<()> {
    let Args {
        image,
        annotation,
        exclude,
    } = args;

    annotations::sizes(&image, &annotation, &exclude).context("Failed to annotate image")?;
    println!("Successfully annotated {image}");

    Ok(())
}

#[cfg(test)]
mod tests {
    use clap::Parser as _;

    use crate::cli::{Args as CliArgs, Command};

    #[test]
    fn annotate_subcommand_parses_annotation_and_excludes() {
        // ARRANGE
        let args = CliArgs::try_parse_from([
            "koci",
            "annotate",
            "--image",
            "repo:test",
            "--annotation",
            "dev.muak.sizes",
            "--exclude",
            "lib/modules",
            "--exclude",
            "usr/share",
        ])
        .expect("parse annotate args");

        // ACT
        let Command::Annotate(annotated) = args.command else {
            panic!("expected annotate command");
        };

        // ASSERT
        assert_eq!(annotated.image, "repo:test");
        assert_eq!(annotated.annotation, "dev.muak.sizes");
        assert_eq!(
            annotated.exclude,
            vec!["lib/modules".to_owned(), "usr/share".to_owned()]
        );
    }
}
