//! Build and push catalog images for a release line.

use std::path::PathBuf;

use anyhow::{Context as _, Result};
use kata::ops::publish;

/// Arguments of the `publish` subcommand.
#[derive(clap::Args, Debug)]
pub struct Args {
    /// Release line to publish (defaults to this binary's version).
    #[arg(long, default_value = kata::version::LINE)]
    release: String,

    /// Channel tags to move to this release (e.g. `stable`, `stable,beta`).
    #[arg(long = "channel", value_name = "CHANNEL", value_delimiter = ',')]
    channel: Vec<String>,

    /// Catalog repository root.
    #[arg(long, value_name = "PATH", default_value = ".")]
    dir: PathBuf,

    /// Replace an already-published line (dev scratch registries only).
    #[arg(long, default_value_t = false)]
    force: bool,
}

/// Execute the `publish` subcommand.
pub(crate) fn run(args: Args) -> Result<()> {
    let Args {
        release,
        channel,
        dir,
        force,
    } = args;

    let published =
        publish::run(&dir, &release, &channel, force).context("Failed to publish catalog")?;
    for digest in published {
        println!("Published {digest}");
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use clap::Parser as _;

    use crate::cli::{Args as CliArgs, Command};

    #[test]
    fn publish_defaults_release_to_the_baked_line() {
        // ARRANGE / ACT
        let args = CliArgs::try_parse_from(["kata", "publish"]).expect("parse");

        // ASSERT
        let Command::Publish(publish) = args.command else {
            panic!("expected publish command");
        };
        assert_eq!(publish.release, kata::version::LINE);
        assert_eq!(publish.channel, Vec::<alloc::string::String>::new());
        assert!(!publish.force);
    }

    #[test]
    fn publish_parses_delimited_channels() {
        // ARRANGE / ACT
        let args = CliArgs::try_parse_from([
            "kata",
            "publish",
            "--release",
            "v1.2.3",
            "--channel",
            "stable,beta",
            "--force",
        ])
        .expect("parse publish args");

        // ASSERT
        let Command::Publish(publish) = args.command else {
            panic!("expected publish command");
        };
        assert_eq!(
            publish.channel,
            vec!["stable".to_owned(), "beta".to_owned()]
        );
        assert!(publish.force);
    }
}
