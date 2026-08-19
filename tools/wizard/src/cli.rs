//! Command-line interface for the wizard.

use std::ffi::OsString;
use std::fs::{self, File};
use std::path::{Path, PathBuf};

use anyhow::{Context as _, Result, bail};
use clap::{Parser, Subcommand};
use koci::arch::Arch;
use sbolt::keys::{SigningPair, load_certificate_from_pem, load_signer_from_pem};

use crate::artifact::Artifact;
use crate::config;
use crate::domain::profile::Profile;
use crate::request::{Platform, Request};
use crate::resolver;

/// Runs the CLI with the given arguments.
///
/// # Errors
///
/// Returns an error when argument parsing or command execution fails.
pub fn run_from<I, T>(args: I) -> Result<()>
where
    I: IntoIterator<Item = T>,
    T: Into<OsString> + Clone,
{
    let args = Cli::parse_from(args);
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

struct BuildArgs {
    profile: PathBuf,
    artifacts: Vec<String>,
    version: String,
    registry: String,
    arch: Arch,
    platform: String,
    output_dir: PathBuf,
    signing_key: Option<PathBuf>,
    signing_cert: Option<PathBuf>,
}

#[derive(Debug, Parser)]
#[command(name = "wizard")]
#[command(about = "Build Muak boot artifacts")]
#[command(version = env!("CARGO_PKG_VERSION"))]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    ProfileId {
        #[arg(short, long)]
        profile: PathBuf,
    },
    Resolve {
        #[arg(short, long)]
        profile: PathBuf,

        #[arg(long)]
        version: String,

        #[arg(long)]
        registry: String,

        #[arg(long, value_parser = crate::arch::parse)]
        arch: Arch,

        #[arg(long)]
        platform: String,
    },
    Build {
        #[arg(short, long)]
        profile: PathBuf,

        #[arg(long, num_args(1..))]
        artifacts: Vec<String>,

        #[arg(long)]
        version: String,

        #[arg(long)]
        registry: String,

        #[arg(long, value_parser = crate::arch::parse)]
        arch: Arch,

        #[arg(long)]
        platform: String,

        #[arg(short, long, default_value = ".")]
        output_dir: PathBuf,

        #[arg(long)]
        signing_key: Option<PathBuf>,

        #[arg(long)]
        signing_cert: Option<PathBuf>,
    },
}

fn parse_platform(input: &str) -> Result<Platform> {
    match input {
        "metal" => Ok(Platform::Metal),
        "aws" => Ok(Platform::Aws),
        "gcp" => Ok(Platform::Gcp),
        _ => Err(anyhow::anyhow!("unknown platform: {input}")),
    }
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

fn run_command(command: Command) -> Result<()> {
    match command {
        Command::ProfileId { profile } => run_profile_id(&profile),
        Command::Resolve {
            profile,
            version,
            registry,
            arch,
            platform,
        } => run_resolve(&profile, &version, &registry, arch, &platform),
        Command::Build {
            profile,
            artifacts,
            version,
            registry,
            arch,
            platform,
            output_dir,
            signing_key,
            signing_cert,
        } => {
            let args = BuildArgs {
                profile,
                artifacts,
                version,
                registry,
                arch,
                platform,
                output_dir,
                signing_key,
                signing_cert,
            };
            run_build(&args)
        }
    }
}

fn run_profile_id(profile_path: &Path) -> Result<()> {
    let bytes = std::fs::read(profile_path)?;
    let spec = Profile::from_toml(&bytes)?;
    println!("{}", spec.profile_id()?);

    Ok(())
}

fn run_resolve(
    profile_path: &Path,
    version: &str,
    registry: &str,
    arch: Arch,
    platform: &str,
) -> Result<()> {
    let bytes = std::fs::read(profile_path)
        .with_context(|| format!("read profile {}", profile_path.display()))?;
    let profile = Profile::from_toml(&bytes)?;
    config::configure(config::Config {
        cache_dir: None,
        registry: registry.to_owned(),
    })?;
    let request = Request::new(version, parse_platform(platform)?).arch(arch);
    let resolved = resolver::plan(&request, &profile)?;

    println!("profile id: {}", resolved.profile_id());
    println!("release id: {}", resolved.release_id());
    println!("resolution id: {}", resolved.resolution_id());
    println!("resolved installer: {}", resolved.build().installer());
    println!(
        "resolved kernel: {} -> {}",
        resolved.build().kernel().image(),
        resolved.build().kernel().source()
    );
    for ext in resolved.build().extensions() {
        println!(" resolved extension: {} -> {}", ext.name(), ext.source());
    }
    if let Some(ov) = resolved.build().overlay() {
        println!(
            "  resolved overlay: {}/{} -> {}",
            ov.image(),
            ov.name(),
            ov.source_ref()
        );
    }

    Ok(())
}

fn run_build(args: &BuildArgs) -> Result<()> {
    let platform = parse_platform(&args.platform)?;

    let artifacts: Vec<Artifact> = args
        .artifacts
        .iter()
        .map(|name| parse_artifact(name))
        .collect::<Result<_>>()?;

    if artifacts.is_empty() {
        bail!("at least one artifact must be specified");
    }

    if args.signing_key.is_some() != args.signing_cert.is_some() {
        bail!("--signing-key and --signing-cert must be provided together");
    }

    let cache_dir = std::env::var_os("HOME").map(|home| Path::new(&home).join(".cache/muak/koci"));
    config::configure(config::Config {
        cache_dir,
        registry: args.registry.clone(),
    })?;

    let profile = build_profile(&args.profile)?;

    let owned_pair = match (args.signing_key.as_ref(), args.signing_cert.as_ref()) {
        (Some(key_path), Some(cert_path)) => {
            let signer = load_signer_from_pem(key_path)?;
            let cert = load_certificate_from_pem(cert_path)?;
            Some((signer, cert))
        }
        (None, None) => None,
        _ => bail!("--signing-key and --signing-cert must be provided together"),
    };

    let signing = match owned_pair {
        Some((ref signer, ref cert)) => Some(SigningPair {
            signer,
            certificate: cert,
        }),
        None => None,
    };

    let mut files: Vec<(Artifact, File)> = Vec::new();
    for &artifact in &artifacts {
        let output_path = args.output_dir.join(artifact.filename());
        let file = File::create(&output_path)
            .with_context(|| format!("create output file {}", output_path.display()))?;
        files.push((artifact, file));
    }

    let mut request = Request::new(&args.version, platform).arch(args.arch);
    let mut remaining: &mut [(Artifact, File)] = &mut files;
    while let Some((first, rest)) = remaining.split_first_mut() {
        let artifact = first.0;
        let file: &mut (dyn std::io::Write + Send) = &mut first.1;
        request = request.artifact(artifact, file)?;
        remaining = rest;
    }

    let request = match signing {
        Some(ref pair) => request.sign(pair),
        None => request,
    };

    let _meta = request.build(&profile).context("build artifacts")?;

    for &artifact in &artifacts {
        let path = args.output_dir.join(artifact.filename());
        let size = fs::metadata(&path).map_or(0, |meta| meta.len());
        println!("Successfully built {} ({} B)", path.display(), size);
    }

    Ok(())
}

fn build_profile(profile_path: &Path) -> Result<Profile> {
    let bytes = std::fs::read(profile_path)
        .with_context(|| format!("read profile {}", profile_path.display()))?;

    Ok(Profile::from_toml(&bytes)?)
}
