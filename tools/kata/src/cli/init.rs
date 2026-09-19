//! Seed a release line.

use std::path::PathBuf;

use anyhow::{Context as _, Result};
use kata::ops::init;

/// Arguments of the `init` subcommand.
#[derive(clap::Args, Debug)]
pub struct Args {
    /// Release line to seed.
    #[arg(long)]
    release: String,

    /// Previous line to copy overlay and extension documents from.
    #[arg(long)]
    from: Option<String>,

    /// Catalog repository root.
    #[arg(long, value_name = "PATH", default_value = ".")]
    dir: PathBuf,
}

/// Execute the `init` subcommand.
pub(crate) fn run(args: Args) -> Result<()> {
    let Args { release, from, dir } = args;

    let written = init::run(&dir, &release, from.as_deref()).context("Failed to seed release")?;
    for path in written {
        println!("Wrote {}", path.display());
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use clap::Parser as _;

    use crate::cli::{Args as CliArgs, Command};

    #[test]
    fn init_parses_release_and_optional_from() {
        // ARRANGE / ACT
        let args =
            CliArgs::try_parse_from(["kata", "init", "--release", "v1.2.3", "--from", "v1.2.2"])
                .expect("parse init args");

        // ASSERT
        let Command::Init(init) = args.command else {
            panic!("expected init command");
        };
        assert_eq!(init.release, "v1.2.3");
        assert_eq!(init.from.as_deref(), Some("v1.2.2"));
        assert_eq!(init.dir.to_str(), Some("."));
    }

    #[test]
    fn init_defaults_from_to_none() {
        // ARRANGE / ACT
        let args = CliArgs::try_parse_from(["kata", "init", "--release", "v1.2.3"]).expect("parse");

        // ASSERT
        let Command::Init(init) = args.command else {
            panic!("expected init command");
        };
        assert_eq!(init.from, None);
    }
}
