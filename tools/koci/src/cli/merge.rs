//! Merge per-platform manifests into a multi-arch index.

use anyhow::{Context as _, Result};
use koci::{error, merge};

/// Arguments of the `merge` subcommand.
#[derive(clap::Args, Debug)]
pub struct Args {
    #[arg(short, long)]
    image: String,

    #[arg(short, long = "tag", required = true)]
    tags: Vec<String>,

    #[arg(value_name = "ARCH=REF", required = true)]
    sources: Vec<String>,
}

/// Execute the `merge` subcommand.
pub(crate) fn run(args: Args) -> Result<()> {
    let Args {
        image,
        tags,
        sources,
    } = args;

    let sources = sources
        .iter()
        .map(|spec| merge::parse_source(spec))
        .collect::<error::Result<Vec<_>>>()
        .context("Failed to parse sources")?;

    merge::index(&image, &tags, &sources).context("Failed to merge index")?;
    println!(
        "Successfully merged {} source(s) into {image}",
        sources.len()
    );

    Ok(())
}

#[cfg(test)]
mod tests {
    use clap::Parser as _;

    use crate::cli::{Args as CliArgs, Command};

    #[test]
    fn merge_subcommand_parses_tags_and_arch_ref_sources() {
        // ARRANGE
        let args = CliArgs::try_parse_from([
            "koci",
            "merge",
            "--image",
            "repo:test",
            "--tag",
            "v1",
            "--tag",
            "latest",
            "amd64=v1-amd64",
            "arm64=sha256:abc",
        ])
        .expect("parse merge args");

        // ACT
        let Command::Merge(merged) = args.command else {
            panic!("expected merge command");
        };

        // ASSERT
        assert_eq!(merged.image, "repo:test");
        assert_eq!(merged.tags, vec!["v1".to_owned(), "latest".to_owned()]);
        assert_eq!(
            merged.sources,
            vec!["amd64=v1-amd64".to_owned(), "arm64=sha256:abc".to_owned()]
        );
    }

    #[test]
    fn merge_requires_at_least_one_tag_and_source() {
        // ARRANGE / ACT / ASSERT
        let error = CliArgs::try_parse_from(["koci", "merge", "--image", "repo:test"])
            .expect_err("merge without tags or sources should not parse");
        assert!(error.to_string().contains("--tag"));

        let error =
            CliArgs::try_parse_from(["koci", "merge", "--image", "repo:test", "--tag", "v1"])
                .expect_err("merge without sources should not parse");
        assert!(error.to_string().contains("ARCH=REF"));
    }
}
