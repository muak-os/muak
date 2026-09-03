//! Command-line interface for the wizard.

use std::ffi::OsString;
use std::fs::{self, File};
use std::path::{Path, PathBuf};

use anyhow::{Context as _, Result, bail};
use clap::{Args, Parser, Subcommand};
use koci::arch::Arch;
use sbolt::keys::{SigningPair, load_certificate_from_pem, load_signer_from_pem};
use wizard::artifact::Artifact;
use wizard::config;
use wizard::domain::profile::Profile;
use wizard::request::{Platform, Request};
use wizard::resolver;

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

        #[arg(long, value_parser = wizard::arch::parse)]
        arch: Arch,

        #[arg(long, value_parser = parse_platform)]
        platform: Platform,
    },
    Build(BuildArgs),
}

/// Arguments of the build subcommand.
#[derive(Debug, Args)]
struct BuildArgs {
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

    #[arg(long, value_parser = parse_platform)]
    platform: Platform,

    #[arg(short, long, default_value = ".")]
    output_dir: PathBuf,

    #[arg(long, requires = "signing_cert")]
    signing_key: Option<PathBuf>,

    #[arg(long, requires = "signing_key")]
    signing_cert: Option<PathBuf>,
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
        } => run_resolve(&profile, &version, &registry, arch, platform),
        Command::Build(args) => run_build(&args),
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
    platform: Platform,
) -> Result<()> {
    let bytes = std::fs::read(profile_path)
        .with_context(|| format!("read profile {}", profile_path.display()))?;
    let profile = Profile::from_toml(&bytes)?;
    config::configure(config::Config {
        cache_dir: None,
        registry: registry.to_owned(),
    })?;
    let request = Request::new(version, platform).arch(arch);
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
    if args.artifacts.is_empty() {
        bail!("at least one artifact must be specified");
    }

    let cache_dir = std::env::var_os("HOME").map(|home| Path::new(&home).join(".cache/muak/koci"));
    config::configure(config::Config {
        cache_dir,
        registry: args.registry.clone(),
    })?;

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

    let mut files: Vec<(Artifact, File)> = Vec::new();
    for &artifact in &args.artifacts {
        let output_path = args.output_dir.join(artifact.filename());
        let file = File::create(&output_path)
            .with_context(|| format!("create output file {}", output_path.display()))?;
        files.push((artifact, file));
    }

    let mut request = Request::new(&args.version, args.platform).arch(args.arch);
    for pair in &mut files {
        let writer: &mut (dyn std::io::Write + Send) = &mut pair.1;
        request = request.artifact(pair.0, writer)?;
    }

    let request = match signing {
        Some(ref pair) => request.sign(pair),
        None => request,
    };

    let _meta = request.build(&profile).context("build artifacts")?;

    for &artifact in &args.artifacts {
        let path = args.output_dir.join(artifact.filename());
        let size = fs::metadata(&path).map_or(0, |meta| meta.len());
        println!("Successfully built {} ({} B)", path.display(), size);
    }

    Ok(())
}
