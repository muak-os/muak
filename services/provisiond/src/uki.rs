//! Unified Kernel Image building and management.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use imager::catalog::extension_archive_name;
use imager::source::model::{ResolvedBuildProfile, ResolvedExtension};

/// Public key for installer image verification.
const SIGNATURE_PUB: &str =
    include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/../../signature.pub"));

/// Path-based UKI component.
pub struct Uki {
    pub stub: PathBuf,
    pub kernel: PathBuf,
    pub initramfs: PathBuf,
    pub cmdline: PathBuf,
    pub dtb: Option<PathBuf>,
    pub luks_key: Option<Vec<u8>>,
}

impl Uki {
    /// Creates a Uki instance by locating components in a base directory.
    pub fn from_dir(base_dir: &Path) -> Self {
        let arch_dir = base_dir.join(std::env::consts::ARCH);
        Self {
            stub: arch_dir.join("stub.efi"),
            kernel: arch_dir.join("vmlinuz"),
            initramfs: arch_dir.join("initramfs.img"),
            cmdline: arch_dir.join("cmdline"),
            dtb: None,
            luks_key: None,
        }
    }

    /// Pulls the installer, builds the merged initramfs via imager, and returns a Uki.
    pub async fn prepare(
        installer_image: &str,
        extensions: &[String],
        work_dir: &Path,
    ) -> Result<Self> {
        let uki = Self::from_dir(work_dir);
        let parent = uki.kernel.parent().context("Invalid UKI path")?;

        std::fs::create_dir_all(parent)
            .with_context(|| format!("Failed to create work dir for {}", work_dir.display()))?;

        let arch = koci::arch::host();
        let resolved = ResolvedBuildProfile::new(
            imager::request::Platform::Metal,
            String::new(),
            arch,
            resolve_extensions(extensions),
            None,
            installer_image.to_owned(),
        );

        let installer_dir = work_dir.join("installer");
        imager::stage::pull_installer(&resolved, &installer_dir, Some(SIGNATURE_PUB))
            .await
            .context("Failed to pull installer")?;

        let assets = imager::stage::load_installer_assets(&installer_dir)
            .context("Failed to load installer assets")?;

        let pulled = if extensions.is_empty() {
            vec![]
        } else {
            imager::stage::pull_extensions(resolved.extensions(), &arch, work_dir, None)
                .await
                .context("Failed to pull extensions")?
        };
        let extra_files = build_extension_entries(&pulled);

        let built = imager::pipeline::build_merged_initramfs(&assets, &extra_files, parent)
            .await
            .context("Failed to build initramfs")?;

        std::fs::copy(&assets.kernel, &uki.kernel)
            .with_context(|| format!("copy kernel to {}", uki.kernel.display()))?;
        std::fs::copy(&assets.cmdline, &uki.cmdline)
            .with_context(|| format!("copy cmdline to {}", uki.cmdline.display()))?;
        std::fs::copy(&assets.stub, &uki.stub)
            .with_context(|| format!("copy stub to {}", uki.stub.display()))?;
        if built != uki.initramfs {
            std::fs::copy(&built, &uki.initramfs)
                .with_context(|| format!("copy initramfs to {}", uki.initramfs.display()))?;
        }

        kmsg::info!("Successfully prepared UKI components");

        Ok(uki)
    }

    /// Builds the UKI binary via imager and optionally signs it for Secure Boot.
    pub async fn build(
        &self,
        output: &Path,
        hierarchy: Option<&sbolt::keys::hierarchy::Bundle>,
    ) -> Result<()> {
        if let Some(parent) = output.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("Failed to create directory {}", parent.display()))?;
        }

        let assets = imager::stage::InstallerAssets {
            kernel: self.kernel.clone(),
            initramfs: self.initramfs.clone(),
            stub: self.stub.clone(),
            cmdline: self.cmdline.clone(),
        };

        let output_dir = output.parent().context("output has no parent")?;
        let uki_path = imager::pipeline::build_uki(&assets, &self.initramfs, output_dir)
            .await
            .context("Failed to build UKI")?;

        if uki_path != output {
            std::fs::copy(&uki_path, output)
                .with_context(|| format!("copy UKI to {}", output.display()))?;
        }

        if let Some(hierarchy) = hierarchy {
            let buffer =
                std::fs::read(output).with_context(|| format!("read UKI {}", output.display()))?;
            let signed = sbolt::pe::signature::sign(
                &buffer,
                &hierarchy.db.signer,
                &hierarchy.db.certificate,
            )
            .context("Failed to sign UKI")?;
            std::fs::write(output, &signed)
                .with_context(|| format!("write signed UKI {}", output.display()))?;
            kmsg::info!("UKI signed successfully");
        }

        kmsg::info!("Successfully built UKI at {}", output.display());

        Ok(())
    }

    /// Reads the UKI component files and returns section data for PCR prediction.
    pub fn read_section_data(&self) -> Result<Vec<(String, Vec<u8>)>> {
        let read = |path: &Path| -> Result<Vec<u8>> {
            std::fs::read(path).with_context(|| format!("Failed to read {}", path.display()))
        };
        Ok(vec![
            (".linux".to_string(), read(&self.kernel)?),
            (".cmdline".to_string(), read(&self.cmdline)?),
            (".initrd".to_string(), read(&self.initramfs)?),
        ])
    }
}

/// Resolves extension OCI refs into `ResolvedExtension` entries.
fn resolve_extensions(refs: &[String]) -> Vec<ResolvedExtension> {
    refs.iter()
        .map(|r| {
            let name = r
                .split('/')
                .last()
                .unwrap_or(r)
                .split(':')
                .next()
                .unwrap_or(r)
                .to_owned();
            ResolvedExtension::new(name, r.clone())
        })
        .collect()
}

/// Builds `ExtraFile` entries from pulled extension directories.
fn build_extension_entries(dirs: &[(String, PathBuf)]) -> Vec<ramune::ExtraFile<'_>> {
    dirs.iter()
        .map(|(name, dir)| ramune::ExtraFile {
            name: format!("extensions/{}.erofs", extension_archive_name(name)),
            path: dir.as_path(),
            compress: true,
        })
        .collect()
}
