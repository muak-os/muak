//! Command-line interface for the imager.

use std::ffi::OsString;
use std::path::PathBuf;

use anyhow::{Context as _, Result};
use clap::{Parser, Subcommand};
use koci::arch::Arch;

use crate::profile::{CustomizationSpec, OverlaySpec, Profile};
use crate::request::{Artifact, Platform, Request};
use crate::source::resolver::{Resolver, Sources};

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

fn parse_arch(s: &str) -> Result<Arch> {
    match s {
        "amd64" => Ok(Arch::Amd64),
        "arm64" => Ok(Arch::Arm64),
        _ => Err(anyhow::anyhow!("unknown arch: {s}")),
    }
}

fn parse_platform(s: &str) -> Result<Platform> {
    match s {
        "metal" => Ok(Platform::Metal),
        "aws" => Ok(Platform::Aws),
        "gcp" => Ok(Platform::Gcp),
        _ => Err(anyhow::anyhow!("unknown platform: {s}")),
    }
}

fn parse_artifact(s: &str) -> Result<Artifact> {
    match s {
        "kernel" => Ok(Artifact::Kernel),
        "initramfs" => Ok(Artifact::Initramfs),
        "cmdline" => Ok(Artifact::Cmdline),
        "uki" => Ok(Artifact::Uki),
        "iso" => Ok(Artifact::Iso),
        "raw" => Ok(Artifact::Raw),
        _ => Err(anyhow::anyhow!("unknown artifact: {s}")),
    }
}

pub async fn run_from<I, T>(args: I) -> Result<()>
where
    I: IntoIterator<Item = T>,
    T: Into<OsString> + Clone,
{
    let args = Cli::parse_from(args);
    run_command(args.command).await
}

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

#[must_use]
pub async fn run() -> i32 {
    run_with(std::env::args_os()).await
}

async fn run_command(command: Command) -> Result<()> {
    match command {
        Command::ProfileId { profile } => {
            let bytes = std::fs::read(&profile)?;
            let spec = Profile::from_toml(&bytes)?;
            let id = spec.profile_id()?;
            println!("{id}");

            Ok(())
        }
        Command::Resolve {
            profile,
            version,
            arch,
            platform,
            registry,
            installer,
        } => {
            let bytes = std::fs::read(&profile)
                .with_context(|| format!("read profile {}", profile.display()))?;
            let spec = Profile::from_toml(&bytes)?;
            let request = Request {
                profile_id: String::new(),
                version,
                artifact: Artifact::Kernel,
                platform: parse_platform(&platform)?,
                arch: parse_arch(&arch)?,
            };
            let resolver = Resolver::new(&Sources {
                registry,
                installer,
            });
            let resolved = resolver.resolve(&request, &spec)?;

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
            let arch = parse_arch(&arch)?;
            let platform = parse_platform(&platform)?;
            let artifact = parse_artifact(&artifact)?;

            let spec = if let Some(ref p) = profile {
                let bytes =
                    std::fs::read(p).with_context(|| format!("read profile {}", p.display()))?;
                Profile::from_toml(&bytes)?
            } else {
                Profile {
                    overlay: overlay_name.map(|name| OverlaySpec {
                        name,
                        image: overlay_image.unwrap_or_default(),
                    }),
                    customization: CustomizationSpec {
                        extensions: extension,
                    },
                }
            };

            let request = Request {
                profile_id: spec.profile_id()?,
                version,
                artifact,
                platform,
                arch,
            };

            let resolver = Resolver::new(&Sources {
                registry,
                installer,
            });
            let resolved = resolver.resolve(&request, &spec)?;

            crate::pipeline::build(&resolved, &output)
                .await
                .context(format!(
                    "build {} to {}",
                    request.artifact,
                    output.display()
                ))?;

            println!(
                "Successfully built {} at {}",
                request.artifact,
                output.display()
            );

            Ok(())
        }
    }
}
