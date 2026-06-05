//! Installer and extension staging helpers.

use std::path::{Path, PathBuf};

use koci::arch::Arch;
use tokio::fs;
use tokio::task::spawn_blocking;

use crate::error::{ImagerError, Result};
use crate::resolve::{ResolvedExtension, ResolvedOverlay, ResolvedProfile};

/// Installer asset paths extracted from the source OCI image.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstallerAssets {
    pub kernel: PathBuf,
    pub initramfs: PathBuf,
    pub stub: PathBuf,
    pub cmdline: PathBuf,
}

/// Pulls the source installer OCI image into a workspace directory.
///
/// # Errors
///
/// Returns an error when the OCI pull fails.
pub async fn pull_installer(
    resolved_profile: &ResolvedProfile,
    installer_dir: &Path,
    signature_public_key: Option<&str>,
) -> Result<()> {
    koci::pull_arch(
        resolved_profile.installer(),
        &resolved_profile.arch(),
        installer_dir,
        signature_public_key,
    )
    .await
    .map_err(|e| ImagerError::BuildError(format!("pull installer: {e}")))?;

    Ok(())
}

/// Pulls each resolved extension OCI image into a stable workspace directory.
///
/// # Errors
///
/// Returns an error when any extension OCI pull fails.
pub async fn pull_extensions(
    extensions: &[ResolvedExtension],
    arch: &Arch,
    workspace_root: &Path,
    signature_public_key: Option<&str>,
) -> Result<Vec<(String, PathBuf)>> {
    let extensions_root = workspace_root.join("extensions");
    fs::create_dir_all(&extensions_root)
        .await
        .map_err(|e| ImagerError::BuildError(format!("create extensions dir: {e}")))?;

    let mut pulled = Vec::with_capacity(extensions.len());
    for ext in extensions {
        let dir = extensions_root.join(ext.name().replace('/', "-"));
        koci::pull_arch(ext.source(), arch, &dir, signature_public_key)
            .await
            .map_err(|e| {
                ImagerError::BuildError(format!("pull extension {}: {e}", ext.source()))
            })?;

        pulled.push((ext.name().to_owned(), dir));
    }

    Ok(pulled)
}

/// Pulls the overlay OCI image and extracts its regular files as boot assets.
///
/// # Errors
///
/// Returns an error when the OCI pull or file collection fails.
pub async fn pull_overlay(
    overlay: &ResolvedOverlay,
    arch: &Arch,
    workspace_root: &Path,
    signature_public_key: Option<&str>,
) -> Result<Vec<esp::EspFile>> {
    let overlay_dir = workspace_root.join("overlay");
    koci::pull_arch(
        overlay.source_ref(),
        arch,
        &overlay_dir,
        signature_public_key,
    )
    .await
    .map_err(|e| ImagerError::BuildError(format!("pull overlay: {e}")))?;

    let overlay_dir_clone = overlay_dir.clone();

    spawn_blocking(move || esp::collect_tree(&overlay_dir_clone))
        .await
        .map_err(|e| ImagerError::BuildError(format!("join overlay asset walk: {e}")))?
        .map_err(|e| ImagerError::BuildError(format!("collect overlay assets: {e}")))
}

/// /// Loads required installer asset paths from the extracted OCI rootfs.
///
/// # Errors
///
/// Returns an error when a required installer file is missing.
pub fn load_installer_assets(installer_dir: &Path) -> Result<InstallerAssets> {
    Ok(InstallerAssets {
        kernel: installer_file(installer_dir, "vmlinuz")?,
        initramfs: installer_file(installer_dir, "initramfs.img")?,
        stub: installer_file(installer_dir, "stub.efi")?,
        cmdline: installer_file(installer_dir, "cmdline")?,
    })
}

/// Returns one required installer file path or a typed error when absent.
fn installer_file(installer_dir: &Path, name: &str) -> Result<PathBuf> {
    let path = installer_dir.join(name);
    if path.is_file() {
        Ok(path)
    } else {
        Err(ImagerError::MissingInstallerFile(name.to_owned()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn load_installer_assets_with_all_files() {
        // ARRANGE
        let dir = tempfile::TempDir::new().expect("tempdir");
        for name in &["vmlinuz", "initramfs.img", "stub.efi", "cmdline"] {
            std::fs::write(dir.path().join(name), b"data").expect("write");
        }

        // ACT
        let assets = load_installer_assets(dir.path()).expect("load");

        // ASSERT
        assert!(assets.kernel.is_file());
        assert!(assets.initramfs.is_file());
        assert!(assets.stub.is_file());
        assert!(assets.cmdline.is_file());
    }

    #[test]
    fn load_installer_assets_missing_file() {
        // ARRANGE
        let dir = tempfile::TempDir::new().expect("tempdir");

        // ACT
        let result = load_installer_assets(dir.path());

        // ASSERT
        let err = result.unwrap_err();
        assert!(matches!(err, ImagerError::MissingInstallerFile(_)));
    }

    #[test]
    fn load_installer_assets_partial() {
        // ARRANGE
        let dir = tempfile::TempDir::new().expect("tempdir");
        std::fs::write(dir.path().join("vmlinuz"), b"data").expect("write");

        // ACT
        let result = load_installer_assets(dir.path());

        // ASSERT
        let _err = result.unwrap_err();
    }
}
