//! Installer OCI image metadata extraction and file pulling.

use std::io::Read;

use koci::arch::Arch;
use koci::error::KociError;
use koci::pull::{
    self,
    entries::{FileEntry, MetadataEntry},
};

use crate::error::{Result, WizardError};

/// Installer metadata extracted from the source OCI image.
pub struct Metadata {
    /// UEFI stub size in bytes.
    pub stub_size: u64,
    /// Kernel command line size in bytes.
    pub cmdline_size: u64,
    /// Kernel image size in bytes.
    pub kernel_size: u64,
    /// Initramfs image size in bytes.
    pub initramfs_size: u64,
}

/// Extracts installer metadata from the source OCI image.
///
/// # Errors
///
/// Returns an error when the OCI metadata extraction fails.
pub async fn metadata(
    installer_ref: &str,
    arch: &Arch,
    signature_public_key: Option<&str>,
) -> Result<Metadata> {
    let mut meta = Metadata {
        stub_size: 0,
        cmdline_size: 0,
        kernel_size: 0,
        initramfs_size: 0,
    };

    pull::metadata(
        installer_ref,
        arch,
        signature_public_key,
        |entry: MetadataEntry| {
            match entry.path.as_str() {
                "stub.efi" => meta.stub_size = entry.size,
                "cmdline" => meta.cmdline_size = entry.size,
                "vmlinuz" => meta.kernel_size = entry.size,
                "initramfs.img" => meta.initramfs_size = entry.size,
                _ => {}
            }
            Ok(())
        },
    )
    .await
    .map_err(|e| WizardError::BuildError(format!("extract installer metadata: {e}")))?;

    if meta.stub_size == 0 {
        return Err(WizardError::MissingInstallerFile("stub.efi".to_owned()));
    }
    if meta.cmdline_size == 0 {
        return Err(WizardError::MissingInstallerFile("cmdline".to_owned()));
    }
    if meta.kernel_size == 0 {
        return Err(WizardError::MissingInstallerFile("vmlinuz".to_owned()));
    }
    if meta.initramfs_size == 0 {
        return Err(WizardError::MissingInstallerFile(
            "initramfs.img".to_owned(),
        ));
    }

    Ok(meta)
}

/// Pulls installer files from the installer OCI image, calling `on_entry` for
/// each file with its path, size, and readable stream.
///
/// # Errors
///
/// Returns an error when the OCI pull fails or the handler returns an error.
pub async fn pull<F>(installer_ref: &str, arch: &Arch, mut on_entry: F) -> Result<()>
where
    F: FnMut(&str, u64, &mut dyn Read) -> std::io::Result<()>,
{
    pull::files(installer_ref, arch, None, |entry: FileEntry| {
        on_entry(&entry.path, entry.size, entry.reader).map_err(KociError::IoError)?;
        Ok(())
    })
    .await
    .map_err(|e| WizardError::BuildError(format!("pull installer files: {e}")))?;

    Ok(())
}
