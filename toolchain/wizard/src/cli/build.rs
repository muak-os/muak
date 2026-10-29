//! Build subcommand.

use std::fs::{self, File};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use anyhow::{Context as _, Result, bail};
use koci::arch::Arch;
use sbolt::keys::{SigningPair, load_certificate_from_pem, load_signer_from_pem};
use tui::live::summary::Summary;
use tui::{progress, units};
use wizard::artifact::Artifact;
use wizard::codec::Codec;
use wizard::config;
use wizard::domain::profile::Profile;
use wizard::request::Request;

use super::console::Console;
use super::writer::CountingWriter;

/// Arguments of the `build` subcommand.
#[derive(Debug, clap::Args)]
pub(super) struct Args {
    #[arg(short, long)]
    profile: PathBuf,

    #[arg(long, num_args(1..), value_parser = parse_artifact)]
    artifacts: Vec<Artifact>,

    #[arg(long)]
    version: String,

    #[arg(long)]
    registry: String,

    #[arg(long, value_parser = wizard::arch::parse)]
    arch: Arch,

    #[arg(long, value_parser = Codec::parse)]
    compression: Option<Codec>,

    #[arg(short, long, default_value = ".")]
    output_dir: PathBuf,

    #[arg(long, requires = "signing_cert")]
    signing_key: Option<PathBuf>,

    #[arg(long, requires = "signing_key")]
    signing_cert: Option<PathBuf>,
}

/// Executes the `build` subcommand.
pub(super) fn run(args: &Args, console: &Console) -> Result<()> {
    if args.artifacts.is_empty() {
        bail!("at least one artifact must be specified");
    }

    let cache_dir = std::env::var_os("HOME").map(|home| Path::new(&home).join(".cache/muak/koci"));
    if let Some(dir) = cache_dir {
        config::init_cache(dir);
    }

    let bytes = std::fs::read(&args.profile)
        .with_context(|| format!("read profile {}", args.profile.display()))?;
    let profile = Profile::from_toml(&bytes)?;

    let owned_pair = match (args.signing_key.as_ref(), args.signing_cert.as_ref()) {
        (Some(key_path), Some(cert_path)) => Some((
            load_signer_from_pem(key_path)?,
            load_certificate_from_pem(cert_path)?,
        )),
        _ => None,
    };
    let signing = owned_pair.as_ref().map(|pair| SigningPair {
        signer: &pair.0,
        certificate: &pair.1,
    });

    let codec = args.compression.unwrap_or_default();
    let started = Instant::now();

    console.tracked(|reporter| {
        let mut files: Vec<(Artifact, CountingWriter<'_>)> = Vec::new();
        for &artifact in &args.artifacts {
            let output_path = args.output_dir.join(artifact.output_name(codec));
            let file = File::create(&output_path)
                .with_context(|| format!("create output file {}", output_path.display()))?;
            files.push((artifact, CountingWriter::new(file, artifact, reporter)));
        }

        let mut request = Request::new(&args.version, &args.registry)
            .arch(args.arch)
            .progress(reporter);
        if let Some(compression) = args.compression {
            request = request.codec(compression);
        }
        for pair in &mut files {
            let writer: &mut (dyn std::io::Write + Send) = &mut pair.1;
            request = request.artifact(pair.0, writer)?;
        }

        let request = match signing {
            Some(ref pair) => request.sign(pair),
            None => request,
        };

        request.build(&profile).context("build artifacts")?;

        Ok(())
    })?;

    report_outputs(args, codec, started.elapsed(), console);

    Ok(())
}

fn parse_artifact(input: &str) -> Result<Artifact> {
    match input {
        "kernel" => Ok(Artifact::Kernel),
        "initramfs" => Ok(Artifact::Initramfs),
        "cmdline" => Ok(Artifact::Cmdline),
        "uki" => Ok(Artifact::Uki),
        "iso" => Ok(Artifact::Iso),
        "raw" => Ok(Artifact::Raw),
        "overlays" => Ok(Artifact::Overlays),
        _ => Err(anyhow::anyhow!("unknown artifact: {input}")),
    }
}

fn report_outputs(args: &Args, codec: Codec, elapsed: Duration, console: &Console) {
    let title = format!(
        "Built {} artifacts in {}",
        args.artifacts.len(),
        progress::clock(elapsed)
    );
    let rows: Vec<(String, String)> = args
        .artifacts
        .iter()
        .map(|&artifact| {
            let path = args.output_dir.join(artifact.output_name(codec));
            let size = fs::metadata(&path).map_or(0, |meta| meta.len());
            (
                artifact.filename().to_owned(),
                format!("{} ({})", path.display(), units::bytes(size)),
            )
        })
        .collect();
    let borrowed: Vec<(&str, &str)> = rows
        .iter()
        .map(|row| (row.0.as_str(), row.1.as_str()))
        .collect();

    console.report(&Summary::new(&title, &borrowed));
}
