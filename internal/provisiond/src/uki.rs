//! Unified Kernel Image building and management.

use std::path::Path;

use anyhow::{Context, Result, bail};

use crate::constants;

#[cfg(target_arch = "x86_64")]
pub const UKI_FILENAME: &str = "BOOTX64.EFI";

#[cfg(target_arch = "aarch64")]
pub const UKI_FILENAME: &str = "BOOTAA64.EFI";

/// Wrapper around yuki::Components to provide additional functionality for UKI management.
pub struct Uki(yuki::Components);

impl std::ops::Deref for Uki {
    type Target = yuki::Components;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl std::ops::DerefMut for Uki {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

impl Uki {
    /// Creates a Uki instance by locating components in a base directory.
    pub fn from_dir(base_dir: &Path) -> Self {
        let arch_dir = base_dir.join(std::env::consts::ARCH);
        Self(yuki::Components {
            stub: arch_dir.join("stub.efi"),
            kernel: arch_dir.join("vmlinuz"),
            initramfs: arch_dir.join("initramfs.img"),
            cmdline: arch_dir.join("cmdline.txt"),
            dtb: None,
            luks_key: None,
        })
    }

    /// Prepares UKI components from an installer image and extensions.
    pub async fn prepare(
        installer_image: &str,
        extensions: &[String],
        work_dir: &Path,
    ) -> Result<Self> {
        let uki = Self::from_dir(work_dir);
        let parent = uki.kernel.parent().context("Invalid UKI path")?;

        std::fs::create_dir_all(parent)
            .with_context(|| format!("Failed to create work dir for {}", work_dir.display()))?;

        pull_installer(installer_image, parent).await?;
        build_initramfs(parent, &uki.initramfs, extensions).await?;
        write_cmdline(&uki.cmdline, constants::DEFAULT_CMDLINE)?;

        Ok(uki)
    }

    /// Builds the UKI binary and optionally signs it for Secure Boot.
    pub fn build(
        &self,
        output: &Path,
        hierarchy: Option<&sbolt::keys::KeyHierarchy>,
    ) -> Result<()> {
        if let Some(parent) = output.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("Failed to create directory {}", parent.display()))?;
        }

        let buffer = yuki::build(&self).context("Failed to build UKI")?;

        let final_buffer = if let Some(hierarchy) = hierarchy {
            let signed = sbolt::pe::sign(&buffer, &hierarchy.db.signer, &hierarchy.db.certificate)
                .context("Failed to sign UKI")?;
            kmsg::info!("UKI signed successfully");
            signed
        } else {
            buffer
        };

        std::fs::write(output, &final_buffer).context("Failed to write the UKI")?;
        kmsg::info!("Successfully built UKI at {}", output.display());

        Ok(())
    }

    /// Reads the UKI component files and returns section data for PCR prediction.
    pub fn read_section_data(&self) -> Result<Vec<(String, Vec<u8>)>> {
        let linux = std::fs::read(&self.kernel)
            .with_context(|| format!("Failed to read {}", self.kernel.display()))?;
        let cmdline = std::fs::read(&self.cmdline)
            .with_context(|| format!("Failed to read {}", self.cmdline.display()))?;
        let initrd = std::fs::read(&self.initramfs)
            .with_context(|| format!("Failed to read {}", self.initramfs.display()))?;

        Ok(vec![
            (".linux".to_string(), linux),
            (".cmdline".to_string(), cmdline),
            (".initrd".to_string(), initrd),
        ])
    }
}

/// Pulls the installer image and extracts components.
async fn pull_installer(image: &str, dest_dir: &Path) -> Result<()> {
    kmsg::info!("Pulling installer image: {}", image);

    imager::pull_image(image, dest_dir)
        .await
        .context("Failed to pull installer image")?;

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
async fn build_initramfs(base_dir: &Path, output: &Path, extensions: &[String]) -> Result<()> {
    let base_initramfs = base_dir.join("base-initramfs.img");

    if !base_initramfs.exists() {
        bail!("Base initramfs not found at {}", base_initramfs.display());
    }

    imager::build_initramfs(&base_initramfs, extensions, output)
        .await
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
