//! Installer and extension staging helpers.

use std::io::Read as _;
use std::path::Path;

use koci::arch::Arch;
use koci::pulled::{PulledEntry, PulledFile, PulledImage};

use crate::error::{Result, WizardError};
use crate::resolve::{ResolvedExtension, ResolvedProfile};

/// Installer asset handles extracted from the source OCI image.
#[derive(Debug, Clone)]
pub struct InstallerAssets {
    pub kernel: PulledFile,
    pub initramfs: PulledFile,
    pub stub: PulledFile,
    pub cmdline: PulledFile,
}

/// Pulls the source installer OCI image into memory.
///
/// # Errors
///
/// Returns an error when the OCI pull fails.
pub async fn pull_installer(
    resolved_profile: &ResolvedProfile,
    signature_public_key: Option<&str>,
) -> Result<PulledImage> {
    koci::pull_arch(
        resolved_profile.installer(),
        &resolved_profile.arch(),
        signature_public_key,
    )
    .await
    .map_err(|e| WizardError::BuildError(format!("pull installer: {e}")))
}

/// Pulls each resolved extension OCI image into memory.
///
/// # Errors
///
/// Returns an error when any extension OCI pull fails.
pub async fn pull_extensions(
    extensions: &[ResolvedExtension],
    arch: &Arch,
    signature_public_key: Option<&str>,
) -> Result<Vec<(String, PulledImage)>> {
    let mut pulled = Vec::with_capacity(extensions.len());
    for ext in extensions {
        let image = koci::pull_arch(ext.source(), arch, signature_public_key)
            .await
            .map_err(|e| {
                WizardError::BuildError(format!("pull extension {}: {e}", ext.source()))
            })?;
        pulled.push((ext.name().to_owned(), image));
    }

    Ok(pulled)
}

/// Pulls the overlay OCI image if the resolved profile specifies one.
///
/// # Errors
///
/// Returns an error when the OCI pull or file collection fails.
pub async fn pull_overlay(resolved_profile: &ResolvedProfile) -> Result<Vec<esp::EspFile>> {
    let Some(overlay) = resolved_profile.overlay() else {
        return Ok(vec![]);
    };
    let image = koci::pull_arch(overlay.source_ref(), &resolved_profile.arch(), None)
        .await
        .map_err(|e| WizardError::BuildError(format!("pull overlay: {e}")))?;

    collect_overlay_files(&image, overlay.name())
}

/// Loads required installer assets from the pulled OCI image.
///
/// # Errors
///
/// Returns an error when a required installer file is missing.
pub fn load_installer_assets(installer: &PulledImage) -> Result<InstallerAssets> {
    Ok(InstallerAssets {
        kernel: installer_file(installer, "vmlinuz")?,
        initramfs: installer_file(installer, "initramfs.img")?,
        stub: installer_file(installer, "stub.efi")?,
        cmdline: installer_file(installer, "cmdline")?,
    })
}

/// Reads all bytes from a pulled file.
///
/// # Errors
///
/// Returns an error when the file stream cannot be read.
pub fn read_file(file: &PulledFile, name: &str) -> Result<Vec<u8>> {
    let mut reader = file
        .open()
        .map_err(|e| WizardError::BuildError(format!("open {name}: {e}")))?;
    let mut bytes = Vec::new();
    reader
        .read_to_end(&mut bytes)
        .map_err(|e| WizardError::BuildError(format!("read {name}: {e}")))?;

    Ok(bytes)
}

fn installer_file(installer: &PulledImage, name: &str) -> Result<PulledFile> {
    installer
        .file(Path::new(name))
        .map_err(|e| WizardError::BuildError(format!("lookup installer file {name}: {e}")))?
        .ok_or_else(|| WizardError::MissingInstallerFile(name.to_owned()))
}

fn collect_overlay_files(image: &PulledImage, overlay_name: &str) -> Result<Vec<esp::EspFile>> {
    let prefix = Path::new(overlay_name);
    let mut files = Vec::new();

    for entry in image
        .entries()
        .map_err(|e| WizardError::BuildError(format!("list overlay entries: {e}")))?
    {
        let PulledEntry::File { path, file } = entry else {
            continue;
        };
        if !path.starts_with(prefix) {
            continue;
        }
        let rel = path
            .strip_prefix(prefix)
            .ok()
            .and_then(|path| path.to_str())
            .map(str::to_owned)
            .filter(|path| !path.is_empty())
            .ok_or_else(|| {
                WizardError::BuildError(format!("invalid overlay path: {}", path.display()))
            })?;
        files.push(esp::EspFile {
            path: rel,
            data: read_file(&file, "overlay file")?,
        });
    }

    files.sort_unstable_by(|left, right| left.path.cmp(&right.path));

    Ok(files)
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use koci::pulled::PulledImage;

    use super::*;

    #[test]
    fn load_installer_assets_with_all_files() {
        // ARRANGE
        let mut image = PulledImage::new();
        for name in ["vmlinuz", "initramfs.img", "stub.efi", "cmdline"] {
            image.add_file(Path::new(name), 0o644, b"data".to_vec());
        }

        // ACT
        let assets = load_installer_assets(&image).expect("load assets");

        // ASSERT
        assert_eq!(assets.kernel.len, 4);
        assert_eq!(assets.initramfs.len, 4);
        assert_eq!(assets.stub.len, 4);
        assert_eq!(assets.cmdline.len, 4);
    }

    #[test]
    fn load_installer_assets_missing_file() {
        // ARRANGE
        let image = PulledImage::new();

        // ACT
        let result = load_installer_assets(&image);

        // ASSERT
        assert!(matches!(result, Err(WizardError::MissingInstallerFile(_))));
    }

    #[test]
    fn collect_overlay_files_filters_by_overlay_name() {
        // ARRANGE
        let mut image = PulledImage::new();
        image.add_file(Path::new("rpi/EFI/BOOT/boot.cfg"), 0o644, b"cfg".to_vec());
        image.add_file(Path::new("other/ignored"), 0o644, b"ignored".to_vec());

        // ACT
        let files = collect_overlay_files(&image, "rpi").expect("collect overlay files");

        // ASSERT
        assert_eq!(files.len(), 1);
        assert_eq!(
            files.first().map(|file| file.path.as_str()),
            Some("EFI/BOOT/boot.cfg")
        );
        assert_eq!(
            files.first().map(|file| file.data.as_slice()),
            Some(b"cfg".as_slice())
        );
    }
}
