//! Command-line interface for koci.

use std::ffi::OsString;
use std::path::{Path, PathBuf};

use anyhow::{Context as _, Result};
use clap::{Parser, Subcommand};
use koci::annotations::{self, Verification};
use koci::arch;
use koci::arch::Arch;
use koci::error;
use koci::pull;

/// Top-level CLI arguments.
#[derive(Parser, Debug)]
#[command(name = env!("CARGO_PKG_NAME"))]
#[command(about = env!("CARGO_PKG_DESCRIPTION"))]
struct Args {
    #[command(subcommand)]
    command: Command,
}

/// Available subcommands.
#[derive(Subcommand, Debug)]
enum Command {
    Pull {
        #[arg(short, long)]
        image: String,

        #[arg(long)]
        arch: Option<Arch>,

        #[arg(short, long)]
        output: PathBuf,

        #[arg(long, value_name = "PATH", requires = "sig_annotation")]
        pub_key: Option<PathBuf>,

        #[arg(long, value_name = "KEY", requires = "pub_key")]
        sig_annotation: Option<String>,
    },
    Sign {
        #[arg(short, long)]
        image: String,

        #[arg(long, value_name = "PATH")]
        key: PathBuf,

        #[arg(long, value_name = "KEY")]
        annotation: String,
    },
    Annotate {
        #[arg(short, long)]
        image: String,

        #[arg(long, value_name = "KEY")]
        annotation: String,

        #[arg(long, value_name = "PREFIX")]
        exclude: Vec<String>,
    },
}

fn read_key_file(path: &Path) -> Result<String> {
    std::fs::read_to_string(path)
        .with_context(|| format!("Failed to read key from {}", path.display()))
}

fn write_entry_to_dir(
    mut entry: pull::entries::FileEntry<'_>,
    output: &std::path::Path,
) -> error::Result<()> {
    let file_path = output.join(&entry.path);
    if let Some(parent) = file_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut file = std::fs::File::create(&file_path)?;
    std::io::copy(&mut entry.reader, &mut file)?;

    Ok(())
}

fn run_command(command: Command) -> Result<()> {
    match command {
        Command::Pull {
            image,
            arch,
            output,
            pub_key,
            sig_annotation,
        } => {
            let pubkey_pem = pub_key
                .as_ref()
                .map(|path| read_key_file(path))
                .transpose()?;

            let verification = pubkey_pem.as_deref().zip(sig_annotation.as_deref()).map(
                |(pubkey_pem, sig_annotation)| Verification {
                    pubkey_pem,
                    sig_annotation,
                },
            );

            let target_arch = arch.unwrap_or(arch::host());

            pull::files(&image, &target_arch, verification.as_ref(), |entry| {
                write_entry_to_dir(entry, &output)
            })
            .context("Failed to stream image")?;

            println!("Successfully extracted image to {}", output.display());

            Ok(())
        }
        Command::Sign {
            image,
            key,
            annotation,
        } => {
            let private_key_pem = read_key_file(&key)?;

            annotations::sign(&image, &private_key_pem, &annotation)
                .context("Failed to sign image")?;
            println!("Successfully signed {image}");

            Ok(())
        }
        Command::Annotate {
            image,
            annotation,
            exclude,
        } => {
            annotations::sizes(&image, &annotation, &exclude)
                .context("Failed to annotate image")?;
            println!("Successfully annotated {image}");

            Ok(())
        }
    }
}

/// Run the CLI from a caller-provided argument iterator.
///
/// # Errors
///
/// Returns an error if argument parsing fails or the requested operation fails.
pub fn run_from<I, T>(args: I) -> Result<()>
where
    I: IntoIterator<Item = T>,
    T: Into<OsString> + Clone,
{
    let args = Args::parse_from(args);
    run_command(args.command)
}

/// Runs the CLI with the given arguments and returns an exit code.
pub fn run_with<I, T>(args: I) -> i32
where
    I: IntoIterator<Item = T>,
    T: Into<OsString> + Clone,
{
    match run_from(args) {
        Ok(()) => 0,
        Err(error) => {
            eprintln!("Error: {error:?}");
            1
        }
    }
}

/// Runs the CLI with `std::env::args_os()` and returns an exit code.
#[must_use]
pub fn run() -> i32 {
    run_with(std::env::args_os())
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use clap::Parser as _;
    use koci::arch::Arch;
    use tempfile::TempDir;

    use super::{Args, Command, read_key_file, run_from, run_with};

    #[test]
    fn pull_subcommand_parses_optional_arch_and_pubkey() {
        // ARRANGE
        let args = Args::try_parse_from([
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
        let command = args.command;

        // ASSERT
        assert!(matches!(
            command,
            Command::Pull {
                image,
                arch,
                output,
                pub_key,
                sig_annotation,
            } if image == "repo:test"
                && matches!(arch, Some(Arch::Arm64))
                && output == Path::new("out")
                && pub_key.as_deref() == Some(Path::new("koci.pub"))
                && sig_annotation.as_deref() == Some("dev.muak.sig")
        ));
    }

    #[test]
    fn pull_requires_sig_annotation_alongside_pub_key() {
        // ARRANGE
        // ACT
        let error = Args::try_parse_from([
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

    #[test]
    fn sign_subcommand_parses_key_path_and_annotation() {
        // ARRANGE
        let args = Args::try_parse_from([
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
        let command = args.command;

        // ASSERT
        assert!(matches!(
            command,
            Command::Sign {
                image,
                key,
                annotation
            } if image == "repo:test"
                && key == Path::new("koci.key")
                && annotation == "dev.muak.sig"
        ));
    }

    #[test]
    fn annotate_subcommand_parses_annotation_and_excludes() {
        // ARRANGE
        let args = Args::try_parse_from([
            "koci",
            "annotate",
            "--image",
            "repo:test",
            "--annotation",
            "dev.muak.sizes",
            "--exclude",
            "lib/modules",
            "--exclude",
            "usr/share",
        ])
        .expect("parse annotate args");

        // ACT
        let command = args.command;

        // ASSERT
        assert!(matches!(
            command,
            Command::Annotate {
                image,
                annotation,
                exclude
            } if image == "repo:test"
                && annotation == "dev.muak.sizes"
                && exclude == vec!["lib/modules".to_owned(), "usr/share".to_owned()]
        ));
    }

    #[test]
    fn read_key_file_reports_missing_path() {
        // ARRANGE
        let workspace = TempDir::new().expect("create temp dir");
        let missing = workspace.path().join("missing.pem");

        // ACT
        let error = read_key_file(&missing).expect_err("read should fail");

        // ASSERT
        assert!(error.to_string().contains("Failed to read key from"));
    }

    #[test]
    fn run_from_reports_missing_sign_key() {
        // ARRANGE
        let workspace = TempDir::new().expect("create temp dir");
        let missing = workspace.path().join("missing.pem");

        // ACT
        let error = run_from([
            "koci",
            "sign",
            "--image",
            "repo:test",
            "--key",
            missing.to_str().expect("missing path must be valid utf-8"),
            "--annotation",
            "dev.muak.sig",
        ])
        .expect_err("run_from should fail");

        // ASSERT
        assert!(error.to_string().contains("Failed to read key from"));
    }

    #[test]
    fn run_with_returns_non_zero_for_errors() {
        // ARRANGE
        let workspace = TempDir::new().expect("create temp dir");
        let missing = workspace.path().join("missing.pem");

        // ACT
        let exit_code = run_with([
            "koci",
            "sign",
            "--image",
            "repo:test",
            "--key",
            missing.to_str().expect("missing path must be valid utf-8"),
            "--annotation",
            "dev.muak.sig",
        ]);

        // ASSERT
        assert_eq!(exit_code, 1);
    }
}
