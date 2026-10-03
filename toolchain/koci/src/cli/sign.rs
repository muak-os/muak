//! Sign an OCI image manifest in the registry.

use std::path::PathBuf;

use anyhow::{Context as _, Result};
use koci::annotations;

use super::read_key_file;

/// Arguments of the `sign` subcommand.
#[derive(clap::Args, Debug)]
pub struct Args {
    #[arg(short, long)]
    image: String,

    #[arg(long, value_name = "PATH")]
    key: PathBuf,

    #[arg(long, value_name = "KEY")]
    annotation: String,
}

/// Execute the `sign` subcommand.
pub(crate) fn run(args: Args) -> Result<()> {
    let Args {
        image,
        key,
        annotation,
    } = args;

    let private_key_pem = read_key_file(&key)?;

    annotations::sign(&image, &private_key_pem, &annotation).context("Failed to sign image")?;
    println!("Successfully signed {image}");

    Ok(())
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use clap::Parser as _;

    use crate::cli::{Args as CliArgs, Command};

    #[test]
    fn sign_subcommand_parses_key_path_and_annotation() {
        // ARRANGE
        let args = CliArgs::try_parse_from([
            "koci",
            "sign",
            "--image",
            "repo:test",
            "--key",
            "koci.key",
            "--annotation",
            "dev.muak.sig",
        ])
        .expect("parse sign args");

        // ACT
        let Command::Sign(signed) = args.command else {
            panic!("expected sign command");
        };

        // ASSERT
        assert_eq!(signed.image, "repo:test");
        assert_eq!(signed.key, Path::new("koci.key"));
        assert_eq!(signed.annotation, "dev.muak.sig");
    }
}
