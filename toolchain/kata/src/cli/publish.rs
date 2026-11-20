//! Build and push catalog images for a release line.

use std::path::PathBuf;

use anyhow::{Context as _, Result};
use kata::publish;
use koci::arch::Arch;

use crate::cli::{parse_arch, registry_prefix};

/// Arguments of the `publish` subcommand.
#[derive(clap::Args, Debug)]
pub(super) struct Args {
    /// Release line to publish.
    #[arg(long)]
    release: String,

    /// Channel tags to move to this release (e.g. `stable`, `stable,beta`).
    #[arg(long = "channel", value_name = "CHANNEL", value_delimiter = ',')]
    channel: Vec<String>,

    /// Catalog repository root.
    #[arg(long, value_name = "PATH", default_value = ".")]
    dir: PathBuf,

    /// Architecture recorded in the image config.
    #[arg(long, default_value = "amd64", value_parser = parse_arch)]
    arch: Arch,

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
        arch,
        force,
    } = args;
    let registry = registry_prefix();

    let published = publish::run(
        &publish::Input {
            dir,
            release,
            channels: channel,
            registry: &registry,
            arch,
            force,
        },
        &kata::progress::Noop,
    )
    .context("Failed to publish catalog")?;
    for published in published {
        println!("Published {}", published.reference);
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use clap::Parser as _;

    use crate::cli::{Args as CliArgs, Command};

    #[test]
    fn publish_requires_release() {
        // ACT / ASSERT
        CliArgs::try_parse_from(["kata", "publish"]).expect_err("release must be required");
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
