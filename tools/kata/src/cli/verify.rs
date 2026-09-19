//! HEAD-verify pinned entries against the registry.

use std::path::PathBuf;

use anyhow::{Context as _, Result};
use kata::ops::verify;

use super::registry_prefix;

/// Arguments of the `verify` subcommand.
#[derive(clap::Args, Debug)]
pub struct Args {
    /// Release line (default: every line with documents).
    #[arg(long)]
    release: Option<String>,

    /// Catalog repository root.
    #[arg(long, value_name = "PATH", default_value = ".")]
    dir: PathBuf,

    /// Registry prefix.
    #[arg(long)]
    registry: Option<String>,
}

/// Execute the `verify` subcommand.
pub(crate) fn run(args: Args) -> Result<()> {
    let Args {
        release,
        dir,
        registry,
    } = args;

    let verified = verify::run(
        &dir,
        release.as_deref(),
        &registry_prefix(registry.as_deref()),
    )
    .context("Verification failed")?;
    for line in verified {
        println!("Verified {line}");
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use clap::Parser as _;

    use crate::cli::{Args as CliArgs, Command};

    #[test]
    fn verify_defaults_release_to_every_line() {
        // ARRANGE / ACT
        let args = CliArgs::try_parse_from(["kata", "verify"]).expect("parse verify args");

        // ASSERT
        let Command::Verify(verify) = args.command else {
            panic!("expected verify command");
        };
        assert_eq!(verify.release, None);
        assert_eq!(verify.dir.to_str(), Some("."));
    }
}
