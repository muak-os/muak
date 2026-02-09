//! Unified Kernel Image (UKI) building and management.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};

use crate::constants;

/// Configuration for UKI preparation.
struct UkiConfig<'a> {
    installer_image: &'a str,
    extensions: &'a [String],
    work_dir: &'a Path,
    cmdline: &'a str,
}

/// Components of a Unified Kernel Image.
pub struct Uki {
    pub kernel: PathBuf,
    pub stub: PathBuf,
    pub initramfs: PathBuf,
    pub cmdline: PathBuf,
}

impl Uki {
    /// Creates a Uki instance by locating components in a base directory.
    pub fn from_dir(base_dir: &Path) -> Self {
        let arch_dir = base_dir.join(std::env::consts::ARCH);
        Self {
            kernel: arch_dir.join("vmlinuz"),
            stub: arch_dir.join("stub.efi"),
            initramfs: arch_dir.join("initramfs.img"),
            cmdline: arch_dir.join("cmdline.txt"),
        }
    }

    /// Prepares UKI components from an installer image and extensions.
    pub fn prepare(installer_image: &str, extensions: &[String], work_dir: &Path) -> Result<Self> {
        let config = UkiConfig {
            installer_image,
            extensions,
            work_dir,
            cmdline: constants::DEFAULT_CMDLINE,
        };

        let uki = Self::from_dir(config.work_dir);
        let parent = uki.kernel.parent().context("Invalid UKI path")?;

        std::fs::create_dir_all(parent).with_context(|| {
            format!(
                "Failed to create work dir for {}",
                &config.work_dir.display()
            )
        })?;

        pull_installer(config.installer_image, parent)?;
        build_initramfs(parent, &uki.initramfs, config.extensions)?;
        write_cmdline(&uki.cmdline, config.cmdline)?;

        Ok(uki)
    }

    /// Builds the UKI binary from components.
    pub fn build(&self, output: &Path) -> Result<()> {
        ensure_parent_exists(output)?;

        let buffer = yuki::build(
            &self.stub,
            &self.kernel,
            &self.initramfs,
            &self.cmdline,
            None,
        )
        .context("Failed to build UKI")?;

        std::fs::write(output, &buffer).context("Failed to write the UKI")?;

        kmsg::info!("Successfully built UKI at {}", output.display());
        Ok(())
    }

    /// Builds the UKI atomically using a temp file and rename.
    pub fn build_atomic(&self, output: &Path) -> Result<()> {
        ensure_parent_exists(output)?;

        let temp_output = get_temp_path(output);
        let buffer = yuki::build(
            &self.stub,
            &self.kernel,
            &self.initramfs,
            &self.cmdline,
            None,
        )
        .context("Failed to build UKI")?;

        std::fs::write(&temp_output, &buffer).context("Failed to write the UKI")?;

        std::fs::rename(&temp_output, output).with_context(|| {
            format!(
                "Failed to atomically rename {} to {}",
                temp_output.display(),
                output.display()
            )
        })?;

        kmsg::info!(
            "Successfully built and atomically installed UKI at {}",
            output.display()
        );
        Ok(())
    }

    /// Sign a UKI file in-place with the given Secure Boot key hierarchy.
    pub fn sign(uki_path: &Path, hierarchy: &sbolt::keys::KeyHierarchy) -> Result<()> {
        let uki_data = std::fs::read(uki_path)
            .with_context(|| format!("Failed to read UKI from {}", uki_path.display()))?;

        let signed = sbolt::pe::sign(&uki_data, &hierarchy.db.signer, &hierarchy.db.certificate)
            .context("Failed to sign UKI")?;

        std::fs::write(uki_path, &signed)
            .with_context(|| format!("Failed to write signed UKI to {}", uki_path.display()))?;

        kmsg::info!("UKI signed successfully");
        Ok(())
    }
}

/// Returns the full path to the UKI on the EFI partition.
pub fn get_uki_path(efi_mount: &Path) -> Result<PathBuf> {
    let filename = get_uki_filename()?;
    Ok(efi_mount.join("EFI").join("BOOT").join(filename))
}

/// Returns the UKI filename based on architecture.
fn get_uki_filename() -> Result<&'static str> {
    match std::env::consts::ARCH {
        "x86_64" => Ok("BOOTX64.EFI"),
        "aarch64" => Ok("BOOTAA64.EFI"),
        other => bail!("Unsupported architecture: {}", other),
    }
}

/// Cleans up the work directory.
pub fn cleanup_dir(work_dir: &Path) -> Result<()> {
    if work_dir.exists() {
        std::fs::remove_dir_all(work_dir)
            .with_context(|| format!("Failed to clean up work dir {}", work_dir.display()))?;
        kmsg::info!("Cleaned up work directory {}", work_dir.display());
    }
    Ok(())
}

/// Generates a temp path for atomic writes.
fn get_temp_path(path: &Path) -> PathBuf {
    let mut temp = path.to_path_buf();
    let filename = temp
        .file_name()
        .map(|f| format!("{}.new", f.to_string_lossy()))
        .unwrap_or_else(|| "BOOT.EFI.new".to_string());
    temp.set_file_name(filename);
    temp
}

/// Ensures the parent directory exists.
fn ensure_parent_exists(path: &Path) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("Failed to create directory {}", parent.display()))?;
    }
    Ok(())
}

/// Pulls the installer image and extracts components.
fn pull_installer(image: &str, dest_dir: &Path) -> Result<()> {
    kmsg::info!("Pulling installer image: {}", image);

    imager::pull_image(image, dest_dir).context("Failed to pull installer image")?;

    verify_installer_files(dest_dir)?;

    kmsg::info!("Successfully pulled and extracted installer");
    Ok(())
}

/// Verifies required installer files are present.
fn verify_installer_files(base_dir: &Path) -> Result<()> {
    let required_files = ["vmlinuz", "stub.efi", "base-initramfs.img"];

    for file in &required_files {
        let path = base_dir.join(file);
        if !path.exists() {
            bail!("Required installer file missing: {}", file);
        }
    }

    Ok(())
}

/// Builds the initramfs with extensions.
fn build_initramfs(base_dir: &Path, output: &Path, extensions: &[String]) -> Result<()> {
    let base_initramfs = base_dir.join("base-initramfs.img");

    if !base_initramfs.exists() {
        bail!("Base initramfs not found at {}", base_initramfs.display());
    }

    imager::build_initramfs(&base_initramfs, extensions, output)
        .context("Failed to build initramfs")?;

    if !output.exists() {
        bail!(
            "imager build completed but output file not found: {}",
            output.display()
        );
    }

    kmsg::info!(
        "Successfully built initramfs with {} extensions",
        extensions.len()
    );
    Ok(())
}

/// Writes the kernel cmdline to file.
fn write_cmdline(path: &Path, cmdline: &str) -> Result<()> {
    std::fs::write(path, cmdline)
        .with_context(|| format!("Failed to write cmdline to {}", path.display()))
}
