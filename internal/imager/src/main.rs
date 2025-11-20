use clap::Parser;
use std::path::PathBuf;

mod extract;
mod manifest;
mod oci;
mod overlay;
mod rebuild;

use extract::{extract_initramfs, unsquash};
use manifest::ExtensionManifest;
use oci::pull_and_extract;
use overlay::overlay_extension;
use rebuild::{rebuild_initramfs, squash};

#[derive(Parser)]
#[command(version, about = "Build custom initramfs with OCI extensions")]
struct Cli {
    #[arg(short, long)]
    base: PathBuf,

    #[arg(short, long)]
    extension: Vec<String>,

    #[arg(short, long)]
    output: PathBuf,
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let cli = Cli::parse();

    let temp = tempfile::tempdir()?;

    let initramfs_dir = temp.path().join("initramfs");
    extract_initramfs(&cli.base, &initramfs_dir)?;

    let rootfs_sqsh = initramfs_dir.join("rootfs.sqsh");
    let base_rootfs = temp.path().join("base_rootfs");
    unsquash(&rootfs_sqsh, &base_rootfs)?;

    for ext_image in &cli.extension {
        let ext_dir = pull_and_extract(ext_image)?;
        let manifest = ExtensionManifest::from_file(&ext_dir.join("manifest.yaml"))?;
        overlay_extension(&ext_dir, &base_rootfs, &manifest)?;
    }

    squash(&base_rootfs, &rootfs_sqsh)?;

    let new_initramfs = temp.path().join("initramfs.img");
    rebuild_initramfs(&initramfs_dir, &new_initramfs)?;

    if let Some(parent) = cli.output.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::copy(&new_initramfs, &cli.output)?;

    Ok(())
}
