//! Convert a published image's layers to eStargz, in place.

use anyhow::{Context as _, Result};
use koci::convert;

/// Arguments of the `convert` subcommand.
#[derive(clap::Parser, Debug)]
pub struct Args {
    /// Image reference with a tag, such as `registry/image:tag`.
    pub image: String,
}

/// Execute the `convert` subcommand.
pub(crate) fn run(args: &Args) -> Result<()> {
    convert::run(&args.image).context("Failed to convert image")
}

#[cfg(test)]
mod tests {
    use clap::Parser as _;

    use crate::cli::{Args as CliArgs, Command};

    #[test]
    fn convert_subcommand_parses_the_image() {
        // ARRANGE
        let args = CliArgs::try_parse_from(["koci", "convert", "registry:5000/image:tag"])
            .expect("parse convert args");

        // ACT
        let Command::Convert(converted) = args.command else {
            panic!("expected convert command");
        };

        // ASSERT
        assert_eq!(converted.image, "registry:5000/image:tag");
    }
}
