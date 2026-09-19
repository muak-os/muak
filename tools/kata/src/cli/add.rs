//! Add or update a pinned entry.

use std::path::PathBuf;

use anyhow::{Context as _, Result};
use kata::ops::add::{self, Input};
use kata::schema::view::Role;

use super::registry_prefix;

/// Arguments of the `add` subcommand.
#[derive(clap::Args, Debug)]
pub struct Args {
    /// Entry kind: kernel, stub, installer, overlays, or extensions.
    #[arg(long)]
    kind: String,

    /// Logical source of the payload repository.
    #[arg(long)]
    source: String,

    /// Repository path relative to the registry prefix.
    #[arg(long)]
    repository: String,

    /// Payload tag to resolve.
    #[arg(long)]
    tag: String,

    /// Logical name.
    #[arg(long)]
    name: Option<String>,

    /// Digest claimed by the announcing payload.
    #[arg(long, value_name = "DIGEST")]
    digest: Option<String>,

    /// Release line to write into.
    #[arg(long)]
    release: String,

    /// Catalog repository root.
    #[arg(long, value_name = "PATH", default_value = ".")]
    dir: PathBuf,

    /// Registry prefix.
    #[arg(long)]
    registry: Option<String>,
}

/// Execute the `add` subcommand.
pub(crate) fn run(args: Args) -> Result<()> {
    let Args {
        kind,
        source,
        repository,
        tag,
        name,
        digest,
        release,
        dir,
        registry,
    } = args;

    let role = Role::parse(&kind).context("Invalid kind")?;
    let input = Input {
        role,
        release,
        name,
        source,
        repository,
        tag,
        claimed_digest: digest,
        registry: registry_prefix(registry.as_deref()),
    };

    let path = add::run(&dir, &input).context("Failed to add entry")?;
    println!("Wrote {}", path.display());

    Ok(())
}

#[cfg(test)]
mod tests {
    use clap::Parser as _;

    use crate::cli::{Args as CliArgs, Command};

    #[test]
    fn add_parses_kind_entry_fields_and_claim() {
        // ARRANGE / ACT
        let args = CliArgs::try_parse_from([
            "kata",
            "add",
            "--kind",
            "overlays",
            "--source",
            "muak-os/sbc-raspberrypi",
            "--repository",
            "sbc/raspberrypi",
            "--tag",
            "v0.4.1",
            "--name",
            "rpi_generic",
            "--digest",
            "sha256:abc",
            "--release",
            "v1.2.3",
        ])
        .expect("parse add args");

        // ASSERT
        let Command::Add(add) = args.command else {
            panic!("expected add command");
        };
        assert_eq!(add.kind, "overlays");
        assert_eq!(add.source, "muak-os/sbc-raspberrypi");
        assert_eq!(add.repository, "sbc/raspberrypi");
        assert_eq!(add.tag, "v0.4.1");
        assert_eq!(add.name.as_deref(), Some("rpi_generic"));
        assert_eq!(add.digest.as_deref(), Some("sha256:abc"));
        assert_eq!(add.release, "v1.2.3");
    }
}
