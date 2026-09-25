//! Remove an entry from a release line.

use anyhow::{Context as _, Result};
use clap::Parser;
use kata::ops::remove;
use kata::schema::view::Role;

/// Arguments of the `remove` subcommand.
#[derive(Parser, Debug)]
pub struct Args {
    /// Entry kind: kernel, overlays or extensions.
    #[arg(long)]
    kind: String,

    /// Name of the entry (overlays and extensions).
    #[arg(long)]
    name: Option<String>,

    /// Source of the kernel entry.
    #[arg(long)]
    source: Option<String>,

    /// Catalog repository root.
    #[arg(long, value_name = "PATH", default_value = ".")]
    dir: std::path::PathBuf,

    /// Release line.
    #[arg(long)]
    release: String,
}

/// Execute the `remove` subcommand.
pub(crate) fn run(args: Args) -> Result<()> {
    let Args {
        kind,
        name,
        source,
        dir,
        release,
    } = args;

    let role = Role::parse(&kind).context("Invalid kind")?;
    let identity = remove::run(&remove::Input {
        dir,
        role,
        name,
        source,
        release,
    })
    .context("Failed to remove entry")?;
    println!("Removed {identity}");

    Ok(())
}

#[cfg(test)]
mod tests {
    use clap::Parser as _;

    use crate::cli::{Args as CliArgs, Command};

    #[test]
    fn remove_subcommand_parses_kind_name_and_release() {
        // ARRANGE
        let args = CliArgs::try_parse_from([
            "kata",
            "remove",
            "--kind",
            "overlays",
            "--name",
            "rpi_5",
            "--release",
            "v1.2.0",
        ])
        .expect("parse remove args");

        // ACT
        let Command::Remove(removed) = args.command else {
            panic!("expected remove command");
        };

        // ASSERT
        assert_eq!(removed.kind, "overlays");
        assert_eq!(removed.name.as_deref(), Some("rpi_5"));
        assert_eq!(removed.release, "v1.2.0");
    }
}
