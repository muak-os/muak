//! Command-line interface for the wizard.

use std::ffi::OsString;
use std::fs::File;
use std::path::{Path, PathBuf};

use anyhow::{Context as _, Result, bail};
use clap::{Parser, Subcommand};
use koci::arch::Arch;
use sbolt::keys::{SigningPair, load_certificate_from_pem, load_signer_from_pem};

use crate::artifact::Artifact;
use crate::config::{self, Sources};
use crate::profile::{CustomizationSpec, OverlaySpec, Profile};
use crate::request::{Platform, Request};
use crate::resolve;

struct BuildArgs {
    profile: Option<PathBuf>,
    artifacts: Vec<String>,
    version: String,
    arch: String,
    platform: String,
    extension: Vec<String>,
    overlay_image: Option<String>,
    overlay_name: Option<String>,
    output_dir: PathBuf,
    registry: String,
    installer: String,
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
        arch: String,

        #[arg(long)]
        platform: String,

        #[arg(long, default_value = "ghcr.io")]
        registry: String,

        #[arg(long, default_value = "muak-os/installer")]
        installer: String,
    },
    Build {
        #[arg(short, long)]
        profile: Option<PathBuf>,

        #[arg(long, num_args(1..))]
        artifacts: Vec<String>,

        #[arg(long)]
        version: String,

        #[arg(long)]
        arch: String,

        #[arg(long)]
        platform: String,

        #[arg(long)]
        extension: Vec<String>,

        #[arg(long)]
        overlay_image: Option<String>,

        #[arg(long)]
        overlay_name: Option<String>,

        #[arg(short, long, default_value = ".")]
        output_dir: PathBuf,

        #[arg(long, default_value = "ghcr.io")]
        registry: String,

        #[arg(long, default_value = "muak-os/installer")]
        installer: String,

        #[arg(long)]
        signing_key: Option<PathBuf>,

        #[arg(long)]
        signing_cert: Option<PathBuf>,
    },
}

fn parse_arch(input: &str) -> Result<Arch> {
    match input {
        "amd64" => Ok(Arch::Amd64),
        "arm64" => Ok(Arch::Arm64),
        _ => Err(anyhow::anyhow!("unknown arch: {input}")),
    }
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
        _ => Err(anyhow::anyhow!("unknown artifact: {input}")),
    }
}

/// Runs the CLI with the given arguments.
///
/// # Errors
///
/// Returns an error when argument parsing or command execution fails.
pub async fn run_from<I, T>(args: I) -> Result<()>
where
    I: IntoIterator<Item = T>,
    T: Into<OsString> + Clone,
{
    let args = Cli::parse_from(args);
    run_command(args.command).await
}

/// Runs the CLI with the given arguments and returns an exit code.
pub async fn run_with<I, T>(args: I) -> i32
where
    I: IntoIterator<Item = T>,
    T: Into<OsString> + Clone,
{
    match run_from(args).await {
        Ok(()) => 0,
        Err(error) => {
            eprintln!("Error: {error:?}");
            1
        }
    }
}

/// Runs the CLI with `std::env::args_os()` and returns an exit code.
#[must_use]
pub async fn run() -> i32 {
    run_with(std::env::args_os()).await
}

async fn run_command(command: Command) -> Result<()> {
    match command {
        Command::ProfileId { profile } => run_profile_id(&profile),
        Command::Resolve {
            profile,
            version,
            arch,
            platform,
            registry,
            installer,
        } => run_resolve(&profile, &version, &arch, &platform, registry, installer),
        Command::Build {
            profile,
            artifacts,
            version,
            arch,
            platform,
            extension,
            overlay_image,
            overlay_name,
            output_dir,
            registry,
            installer,
            signing_key,
            signing_cert,
        } => {
            run_build(BuildArgs {
                profile,
                artifacts,
                version,
                arch,
                platform,
                extension,
                overlay_image,
                overlay_name,
                output_dir,
                registry,
                installer,
                signing_key,
                signing_cert,
            })
            .await
        }
    }
}

fn run_profile_id(profile_path: &Path) -> Result<()> {
    let bytes = std::fs::read(profile_path)?;
    let spec = Profile::from_toml(&bytes)?;
    let id = spec.id()?;
    println!("{id}");

    Ok(())
}

fn run_resolve(
    profile_path: &Path,
    version: &str,
    arch: &str,
    platform: &str,
    registry: String,
    installer: String,
) -> Result<()> {
    let bytes = std::fs::read(profile_path)
        .with_context(|| format!("read profile {}", profile_path.display()))?;
    let profile = Profile::from_toml(&bytes)?;
    config::configure(config::Config {
        sources: Sources {
            registry,
            installer,
        },
        cache_dir: None,
    })?;
    let request = Request::new(version, parse_platform(platform)?).arch(parse_arch(arch)?);
    let resolved = resolve::plan(&request, &profile)?;

    println!("resolved installer: {}", resolved.installer());
    for ext in resolved.extensions() {
        println!(" resolved extension: {} -> {}", ext.name(), ext.source());
    }
    if let Some(ov) = resolved.overlay() {
        println!(
            "  resolved overlay: {}/{} -> {}",
            ov.image(),
            ov.name(),
            ov.source_ref()
        );
    }

    Ok(())
}

async fn run_build(args: BuildArgs) -> Result<()> {
    let arch = parse_arch(&args.arch)?;
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
        sources: Sources {
            registry: args.registry,
            installer: args.installer,
        },
        cache_dir,
    })?;

    let profile = build_profile(
        args.profile,
        args.extension,
        args.overlay_image,
        args.overlay_name,
    )?;

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

    let mut request = Request::new(&args.version, platform).arch(arch);
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

    let _meta = request.build(&profile).await.context("build artifacts")?;

    for &artifact in &artifacts {
        let path = args.output_dir.join(artifact.filename());
        println!("Successfully built {}", path.display(),);
    }

    Ok(())
}

fn build_profile(
    profile_path: Option<PathBuf>,
    extensions: Vec<String>,
    overlay_image: Option<String>,
    overlay_name: Option<String>,
) -> Result<Profile> {
    if let Some(path) = profile_path {
        let bytes =
            std::fs::read(&path).with_context(|| format!("read profile {}", path.display()))?;

        Ok(Profile::from_toml(&bytes)?)
    } else {
        let overlay = if let Some(name) = overlay_name {
            Some(OverlaySpec::new(name, overlay_image.unwrap_or_default())?)
        } else {
            None
        };
        let customization = CustomizationSpec::new(extensions)?;

        Ok(Profile::new(overlay, customization))
    }
}
