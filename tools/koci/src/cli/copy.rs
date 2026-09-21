//! Copy images between registries byte-for-byte.

use anyhow::{Context as _, Result};
use koci::copy;

/// Arguments of the `copy` subcommand.
#[derive(clap::Args, Debug)]
pub struct Args {
    /// Source image reference, including its tag.
    #[arg(long)]
    source: String,

    /// Destination image reference, including its tag.
    #[arg(long)]
    destination: String,
}

/// Execute the `copy` subcommand.
pub(crate) fn run(args: Args) -> Result<()> {
    let Args {
        source,
        destination,
    } = args;

    copy::run(&source, &destination).context("Failed to copy image")?;
    println!("Successfully copied {source} to {destination}");

    Ok(())
}

#[cfg(test)]
mod tests {
    use clap::Parser as _;

    use crate::cli::{Args as CliArgs, Command};

    #[test]
    fn copy_subcommand_parses_source_and_destination() {
        // ARRANGE
        let args = CliArgs::try_parse_from([
            "koci",
            "copy",
            "--source",
            "ghcr.io/muak-os/linux:v6.12.4-muak1",
            "--destination",
            "localhost:5000/linux:v6.12.4-muak1",
        ])
        .expect("parse copy args");

        // ACT
        let Command::Copy(copied) = args.command else {
            panic!("expected copy command");
        };

        // ASSERT
        assert_eq!(copied.source, "ghcr.io/muak-os/linux:v6.12.4-muak1");
        assert_eq!(copied.destination, "localhost:5000/linux:v6.12.4-muak1");
    }
}
