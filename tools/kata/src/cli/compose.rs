//! Compose a release line from carried and selected payloads.

use anyhow::{Context as _, Result};
use clap::Parser;
use kata::ops::compose::plan::Selections;
use kata::ops::compose::{self, Mode, Policy};

/// Arguments of the `compose` subcommand.
#[derive(Parser, Debug)]
pub struct Args {
    /// Release line to compose.
    #[arg(long)]
    release: String,

    /// Output docs root.
    #[arg(long, value_name = "PATH", default_value = ".")]
    dir: std::path::PathBuf,

    /// Derivation root whose newest line provides the topology and pins.
    #[arg(long, value_name = "PATH")]
    from: Option<std::path::PathBuf>,

    /// Lineage parent line.
    #[arg(long)]
    from_line: Option<String>,

    /// Retag planned entries: KEY=TAG where KEY is a role shorthand.
    #[arg(long = "set", value_name = "KEY=TAG")]
    sets: Vec<String>,

    /// Bump entries without an explicit selection to their newest version tag.
    #[arg(long, default_value_t = false)]
    auto: bool,

    /// Print the resolved plan as JSON and exit without writing.
    #[arg(long, default_value_t = false)]
    print_plan: bool,

    /// Registry prefix for resolution and publication checks.
    #[arg(long)]
    registry: Option<String>,

    #[command(flatten)]
    flags: Flags,
}

/// Flags controlling gates and scratch behavior.
#[derive(clap::Args, Debug)]
pub struct Flags {
    /// Ignore carried and scratch pins; compose only selected roles.
    #[arg(long, default_value_t = false)]
    fresh: bool,

    /// Replace an already-published line (dev scratch registries only).
    #[arg(long, default_value_t = false)]
    force: bool,

    /// Compose from a line that is not the newest one.
    #[arg(long, default_value_t = false)]
    allow_outdated_from: bool,
}

/// Execute the `compose` subcommand.
pub(crate) fn run(args: Args) -> Result<()> {
    let Args {
        release,
        dir,
        from,
        from_line,
        sets,
        auto,
        print_plan,
        registry,
        flags:
            Flags {
                fresh,
                force,
                allow_outdated_from,
            },
    } = args;

    let selections = Selections {
        sets: parse_pairs(&sets).context("Invalid set selection")?,
    };

    let mode = if print_plan { Mode::Print } else { Mode::Write };
    let written = compose::run(&compose::Input {
        dir,
        from,
        from_line,
        release,
        selections,
        policy: if auto { Policy::Auto } else { Policy::Carried },
        fresh,
        force,
        allow_outdated_from,
        mode,
        registry: super::registry_prefix(registry.as_deref()),
    })
    .context("Failed to compose line")?;

    for path in written {
        println!("Wrote {path}");
    }

    Ok(())
}

fn parse_pairs(specs: &[String]) -> Result<Vec<(String, String)>> {
    specs
        .iter()
        .map(|spec| {
            let (key, tag) = spec
                .split_once('=')
                .ok_or_else(|| anyhow::anyhow!("expected KEY=TAG, got '{spec}'"))?;
            if key.is_empty() || tag.is_empty() {
                return Err(anyhow::anyhow!("expected KEY=TAG, got '{spec}'"));
            }

            Ok((key.to_owned(), tag.to_owned()))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use clap::Parser as _;

    use super::parse_pairs;
    use crate::cli::{Args as CliArgs, Command};

    #[test]
    fn compose_subcommand_parses_sets() {
        // ARRANGE
        let args = CliArgs::try_parse_from([
            "kata",
            "compose",
            "--release",
            "v1.1.0",
            "--from-line",
            "v1.0.0-beta",
            "--set",
            "installer=v1.1.0",
            "--set",
            "overlays/rpi_generic=v0.4.1",
            "--auto",
        ])
        .expect("parse compose args");

        // ACT
        let Command::Compose(composed) = args.command else {
            panic!("expected compose command");
        };

        // ASSERT
        assert_eq!(composed.release, "v1.1.0");
        assert_eq!(composed.from_line.as_deref(), Some("v1.0.0-beta"));
        assert!(composed.auto);
        assert_eq!(
            composed.sets,
            vec![
                "installer=v1.1.0".to_owned(),
                "overlays/rpi_generic=v0.4.1".to_owned()
            ]
        );
    }

    #[test]
    fn parse_pairs_rejects_malformed_selections() {
        // ACT / ASSERT
        parse_pairs(&["rpi_generic".to_owned()]).expect_err("missing tag must fail");
        parse_pairs(&["=v1".to_owned()]).expect_err("missing key must fail");
        parse_pairs(&["rpi_generic=".to_owned()]).expect_err("empty tag must fail");
    }
}
