//! Compose a release line from carried and selected payloads.

use anyhow::{Context as _, Result};
use clap::Parser;
use kata::ops::compose::plan::Selections;
use kata::ops::compose::{self, Policy};

/// Arguments of the `compose` subcommand.
#[derive(Parser, Debug)]
pub struct Args {
    /// Release line to compose (defaults to this binary's version).
    #[arg(long, default_value = kata::version::LINE)]
    release: String,

    /// Output docs root.
    #[arg(long, value_name = "PATH", default_value = ".")]
    dir: std::path::PathBuf,

    /// Lineage parent line (default: newest line other than the release).
    #[arg(long)]
    from_line: Option<String>,

    /// Retag planned entries: KEY=TAG where KEY is a role shorthand.
    #[arg(long = "set", value_name = "KEY=TAG")]
    sets: Vec<String>,

    /// Bump entries without an explicit selection to their newest version tag.
    #[arg(long, default_value_t = false)]
    auto: bool,

    /// Replace an already-published line (dev scratch registries only).
    #[arg(long, default_value_t = false)]
    force: bool,
}

/// Execute the `compose` subcommand.
pub(crate) fn run(args: Args) -> Result<()> {
    let Args {
        release,
        dir,
        from_line,
        sets,
        auto,
        force,
    } = args;

    let selections = Selections {
        sets: parse_pairs(&sets).context("Invalid set selection")?,
    };

    let written = compose::run(&compose::Input {
        dir,
        from_line,
        release,
        selections,
        policy: if auto { Policy::Auto } else { Policy::Carried },
        force,
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
    fn compose_defaults_release_to_the_baked_line() {
        // ARRANGE / ACT
        let args = CliArgs::try_parse_from(["kata", "compose"]).expect("parse compose args");

        // ASSERT
        let Command::Compose(composed) = args.command else {
            panic!("expected compose command");
        };
        assert_eq!(composed.release, kata::version::LINE);
    }

    #[test]
    fn parse_pairs_rejects_malformed_selections() {
        // ACT / ASSERT
        parse_pairs(&["rpi_generic".to_owned()]).expect_err("missing tag must fail");
        parse_pairs(&["=v1".to_owned()]).expect_err("missing key must fail");
        parse_pairs(&["rpi_generic=".to_owned()]).expect_err("empty tag must fail");
    }
}
