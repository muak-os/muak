//! Extract the files of an OCI image.

use std::path::{Path, PathBuf};

use anyhow::{Context as _, Result};
use koci::error;
use koci::pull;
use koci::signature::Verification;
use oci::arch;
use oci::arch::Arch;

use super::{parse_arch, read_key_file};

/// Arguments of the `pull` subcommand.
#[derive(clap::Args, Debug)]
pub struct Args {
    #[arg(short, long)]
    image: String,

    #[arg(long, value_parser = parse_arch)]
    arch: Option<Arch>,

    #[arg(short, long)]
    output: PathBuf,

    #[arg(long, value_name = "PATH", requires = "sig_annotation")]
    pub_key: Option<PathBuf>,

    #[arg(long, value_name = "KEY", requires = "pub_key")]
    sig_annotation: Option<String>,
}

/// Execute the `pull` subcommand.
pub(crate) fn run(args: Args) -> Result<()> {
    let Args {
        image,
        arch,
        output,
        pub_key,
        sig_annotation,
    } = args;

    let pubkey_pem = pub_key
        .as_ref()
        .map(|path| read_key_file(path))
        .transpose()?;

    let verification =
        pubkey_pem
            .as_deref()
            .zip(sig_annotation.as_deref())
            .map(|(pubkey_pem, sig_annotation)| Verification {
                pubkey_pem,
                sig_annotation,
            });

    let target_arch = arch.unwrap_or(arch::host());

    pull::files(&image, &target_arch, verification.as_ref(), |entry| {
        write_entry_to_dir(entry, &output)
    })
    .context("Failed to stream image")?;

    println!("Successfully extracted image to {}", output.display());

    Ok(())
}

fn write_entry_to_dir(mut entry: pull::entries::FileEntry<'_>, output: &Path) -> error::Result<()> {
    let file_path = output.join(&entry.path);
    if let Some(parent) = file_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut file = std::fs::File::create(&file_path)?;
    std::io::copy(&mut entry.reader, &mut file)?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use clap::Parser as _;
    use oci::arch::Arch;

    use crate::cli::{Args as CliArgs, Command};

    #[test]
    fn pull_subcommand_parses_optional_arch_and_pubkey() {
        // ARRANGE
        let args = CliArgs::try_parse_from([
            "koci",
            "pull",
            "--image",
            "repo:test",
            "--arch",
            "arm64",
            "--output",
            "out",
            "--pub-key",
            "koci.pub",
            "--sig-annotation",
            "dev.muak.sig",
        ])
        .expect("parse pull args");

        // ACT
        let Command::Pull(pulled) = args.command else {
            panic!("expected pull command");
        };

        // ASSERT
        assert_eq!(pulled.image, "repo:test");
        assert!(matches!(pulled.arch, Some(Arch::Arm64)));
        assert_eq!(pulled.output, Path::new("out"));
        assert_eq!(pulled.pub_key.as_deref(), Some(Path::new("koci.pub")));
        assert_eq!(pulled.sig_annotation.as_deref(), Some("dev.muak.sig"));
    }

    #[test]
    fn pull_requires_sig_annotation_alongside_pub_key() {
        // ARRANGE / ACT
        let error = CliArgs::try_parse_from([
            "koci",
            "pull",
            "--image",
            "repo:test",
            "--output",
            "out",
            "--pub-key",
            "koci.pub",
        ])
        .expect_err("pull without sig annotation should not parse");

        // ASSERT
        assert!(error.to_string().contains("--sig-annotation"));
    }
}
