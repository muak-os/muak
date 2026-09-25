//! Build and push catalog images for a release line.

use std::path::PathBuf;

use anyhow::{Context as _, Result};
use kata::ops::publish;
use kata::schema::kinds::Kind;
use koci::arch::Arch;

use super::{parse_arch, registry_prefix};

/// Arguments of the `publish` subcommand.
#[derive(clap::Args, Debug)]
pub struct Args {
    /// Release line to publish.
    #[arg(long)]
    release: String,

    /// Catalog kind.
    #[arg(long)]
    kind: Option<String>,

    /// Catalog repository root.
    #[arg(long, value_name = "PATH", default_value = ".")]
    dir: PathBuf,

    /// Registry prefix.
    #[arg(long)]
    registry: Option<String>,

    /// Replace an already-published line (dev scratch registries only).
    #[arg(long, default_value_t = false)]
    force: bool,

    /// Architecture recorded in the image config.
    #[arg(long, default_value = "amd64", value_parser = parse_arch)]
    arch: Arch,
}

/// Execute the `publish` subcommand.
pub(crate) fn run(args: Args) -> Result<()> {
    let Args {
        release,
        kind,
        dir,
        registry,
        arch,
        force,
    } = args;

    let kind = kind
        .map(|name| Kind::parse(&name))
        .transpose()
        .context("Invalid kind")?;
    let published = publish::run(
        &dir,
        &release,
        kind,
        &registry_prefix(registry.as_deref()),
        arch,
        force,
    )
    .context("Failed to publish catalog")?;
    for digest in published {
        println!("Published {digest}");
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use clap::Parser as _;
    use koci::arch::Arch;

    use crate::cli::{Args as CliArgs, Command};

    #[test]
    fn publish_defaults_arch_and_kind() {
        // ARRANGE / ACT
        let args =
            CliArgs::try_parse_from(["kata", "publish", "--release", "v1.2.3"]).expect("parse");

        // ASSERT
        let Command::Publish(publish) = args.command else {
            panic!("expected publish command");
        };
        assert_eq!(publish.release, "v1.2.3");
        assert_eq!(publish.kind, None);
        assert_eq!(publish.arch, Arch::Amd64);
    }

    #[test]
    fn publish_parses_kind_and_arch() {
        // ARRANGE / ACT
        let args = CliArgs::try_parse_from([
            "kata",
            "publish",
            "--release",
            "v1.2.3",
            "--kind",
            "overlays",
            "--arch",
            "arm64",
        ])
        .expect("parse publish args");

        // ASSERT
        let Command::Publish(publish) = args.command else {
            panic!("expected publish command");
        };
        assert_eq!(publish.kind.as_deref(), Some("overlays"));
        assert_eq!(publish.arch, Arch::Arm64);
    }
}
