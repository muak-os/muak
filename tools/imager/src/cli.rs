//! Command-line interface for the imager.

use std::ffi::OsString;
use std::path::{Path, PathBuf};

use anyhow::{Context as _, Result};
use clap::{Parser, Subcommand};
use koci::arch::Arch;

use crate::artifact::Artifact;
use crate::build::{self, Config};
use crate::profile::{CustomizationSpec, OverlaySpec, Profile};
use crate::request::{Build, Platform, Resolve};
use crate::resolve::{self, Sources};

#[derive(Debug, Parser)]
#[command(name = "imager")]
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

        #[arg(long)]
        artifact: String,

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

        #[arg(short, long)]
        output: PathBuf,

        #[arg(long, default_value = "ghcr.io")]
        registry: String,

        #[arg(long, default_value = "muak-os/installer")]
        installer: String,
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
            artifact,
            version,
            arch,
            platform,
            extension,
            overlay_image,
            overlay_name,
            output,
            registry,
            installer,
        } => {
            run_build(
                profile,
                artifact,
                version,
                arch,
                platform,
                extension,
                overlay_image,
                overlay_name,
                output,
                registry,
                installer,
            )
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
    let spec = Profile::from_toml(&bytes)?;
    let request = Resolve {
        version: version.to_owned(),
        platform: parse_platform(platform)?,
        arch: parse_arch(arch)?,
    };
    let sources = Sources {
        registry,
        installer,
    };
    let resolved = resolve::profile(&request, &spec, &sources)?;

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

#[expect(
    clippy::too_many_arguments,
    reason = "CLI arguments are passed individually for clarity"
)]
async fn run_build(
    profile_path: Option<PathBuf>,
    artifact: String,
    version: String,
    arch: String,
    platform: String,
    extension: Vec<String>,
    overlay_image: Option<String>,
    overlay_name: Option<String>,
    output: PathBuf,
    registry: String,
    installer: String,
) -> Result<()> {
    let arch = parse_arch(&arch)?;
    let platform = parse_platform(&platform)?;
    let artifact = parse_artifact(&artifact)?;

    let spec = build_profile(profile_path, extension, overlay_image, overlay_name)?;

    let request = Build {
        version,
        platform,
        arch,
        artifacts: vec![artifact],
    };

    let config = Config {
        sources: Sources {
            registry,
            installer,
        },
        workspace_root: output.clone(),
    };

    let results = build::artifacts(&request, &spec, &config, &output)
        .await
        .context(format!("build {} to {}", artifact, output.display()))?;

    let path = results
        .get(&artifact)
        .context("built artifact not found in results")?;

    println!("Successfully built {} at {}", artifact, path.display());

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
