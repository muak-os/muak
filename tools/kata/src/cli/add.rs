//! Add or update a pinned entry.

use std::path::PathBuf;

use anyhow::{Context as _, Result};
use kata::ops::add::{self, Input};
use kata::repository;
use kata::schema::kinds::Kind;
use kata::schema::view::Role;
use kata::version;

/// Arguments of the `add` subcommand.
#[derive(clap::Args, Debug)]
pub struct Args {
    /// Entry role: kernel, stub, installer, overlays, or extensions.
    #[arg(long)]
    role: String,

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

    /// Release line to write into (defaults to this binary's version).
    #[arg(long, default_value = kata::version::LINE)]
    release: String,

    /// Bypass the version-correlation gate.
    #[arg(long, default_value_t = false)]
    force: bool,

    /// Catalog repository root.
    #[arg(long, value_name = "PATH", default_value = ".")]
    dir: PathBuf,
}

/// Execute the `add` subcommand.
pub(crate) fn run(args: Args) -> Result<()> {
    let Args {
        role,
        source,
        repository,
        tag,
        name,
        release,
        dir,
        force,
    } = args;

    let role = Role::parse(&role).context("Invalid role")?;
    version::ensure_line(
        &release,
        repository::document_path(Kind::Core, &dir, &release).exists(),
        force,
    )?;
    let input = Input {
        role,
        release,
        name,
        source,
        repository,
        tag,
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
    fn add_parses_role_and_entry_fields() {
        // ARRANGE / ACT
        let args = CliArgs::try_parse_from([
            "kata",
            "add",
            "--role",
            "overlays",
            "--source",
            "muak-os/sbc-raspberrypi",
            "--repository",
            "sbc/raspberrypi",
            "--tag",
            "v0.4.1",
            "--name",
            "rpi_generic",
            "--release",
            "v1.2.3",
        ])
        .expect("parse add args");

        // ASSERT
        let Command::Add(add) = args.command else {
            panic!("expected add command");
        };
        assert_eq!(add.role, "overlays");
        assert_eq!(add.source, "muak-os/sbc-raspberrypi");
        assert_eq!(add.repository, "sbc/raspberrypi");
        assert_eq!(add.tag, "v0.4.1");
        assert_eq!(add.name.as_deref(), Some("rpi_generic"));
        assert_eq!(add.release, "v1.2.3");
    }

    #[test]
    fn add_defaults_release_and_parses_force() {
        // ARRANGE / ACT
        let args = CliArgs::try_parse_from([
            "kata",
            "add",
            "--role",
            "kernel",
            "--source",
            "muak-os/linux",
            "--repository",
            "linux",
            "--tag",
            "v6.12.4-muak1",
            "--force",
        ])
        .expect("parse add args");

        // ASSERT
        let Command::Add(add) = args.command else {
            panic!("expected add command");
        };
        assert_eq!(add.release, kata::version::LINE);
        assert!(add.force);
    }
}
