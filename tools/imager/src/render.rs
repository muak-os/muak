//! Artifact rendering functions for staged installer inputs.

use std::io::Cursor;
use std::path::{Path, PathBuf};

use esp::Arch as EspArch;
use esp::EspSpecBuilder;
use koci::arch::Arch;
use miso::error::MisoError;
use tokio::fs;
use tokio::task::spawn_blocking;

use crate::artifact::Artifact;
use crate::catalog::extension_archive_name;
use crate::error::{ImagerError, Result};
use crate::source::ResolvedBuildProfile;
use crate::stage::{self, InstallerAssets};

/// Builds the merged initramfs artifact from base image and extra files.
///
/// # Errors
///
/// Returns an error when ramune fails or output cannot be written.
pub async fn build_merged_initramfs(
    installer_assets: &InstallerAssets,
    extra_files: &[ramune::ExtraFile<'_>],
    output_dir: &Path,
) -> Result<PathBuf> {
    let output_path = output_dir.join(Artifact::Initramfs.filename());
    let config = ramune::ExtendConfig {
        base: &installer_assets.initramfs,
        extra_files,
        compression_level: ramune::DEFAULT_ZSTD_COMPRESSION_LEVEL,
    };

    ramune::extend(&config, &output_path)
        .await
        .map_err(|e| ImagerError::BuildError(format!("build initramfs: {e}")))?;

    Ok(output_path)
}

/// Builds the boot/install UKI and writes it to the output directory.
///
/// # Errors
///
/// Returns an error when reading assets, running yuki, or writing output fails.
pub async fn build_uki(
    installer_assets: &InstallerAssets,
    initramfs_path: &Path,
    output_dir: &Path,
) -> Result<PathBuf> {
    let output_path = output_dir.join(Artifact::Uki.filename());
    let stub = fs::read(&installer_assets.stub)
        .await
        .map_err(|e| ImagerError::BuildError(format!("read stub: {e}")))?;
    let kernel = fs::read(&installer_assets.kernel)
        .await
        .map_err(|e| ImagerError::BuildError(format!("read kernel: {e}")))?;
    let initramfs = fs::read(initramfs_path)
        .await
        .map_err(|e| ImagerError::BuildError(format!("read initramfs: {e}")))?;
    let cmdline = fs::read(&installer_assets.cmdline)
        .await
        .map_err(|e| ImagerError::BuildError(format!("read cmdline: {e}")))?;

    let uki_bytes = spawn_blocking(move || {
        yuki::build(&yuki::BuildInput {
            stub: &stub,
            kernel: &kernel,
            initramfs: &initramfs,
            cmdline: &cmdline,
            dtb: None,
            luks_key: None,
        })
        .map_err(|e| ImagerError::BuildError(format!("build UKI: {e}")))
    })
    .await
    .map_err(|e| ImagerError::BuildError(format!("join UKI build task: {e}")))?
    .map_err(|e| ImagerError::BuildError(format!("build UKI: {e}")))?;

    fs::write(&output_path, uki_bytes)
        .await
        .map_err(|e| ImagerError::BuildError(format!("write UKI: {e}")))?;

    Ok(output_path)
}

/// Builds the bootable ISO artifact from a prebuilt UKI.
///
/// # Errors
///
/// Returns an error when reading the UKI, running miso, or writing output fails.
pub async fn build_iso(
    resolved_profile: &ResolvedBuildProfile,
    output_dir: &Path,
    uki_path: &Path,
) -> Result<PathBuf> {
    let uki_bytes = fs::read(uki_path)
        .await
        .map_err(|e| ImagerError::BuildError(format!("read UKI: {e}")))?;
    let arch = esp_arch(resolved_profile.arch());
    let spec = EspSpecBuilder::default()
        .with_uki(arch, uki_bytes)
        .map_err(|e| ImagerError::BuildError(format!("add UKI to ISO ESP spec: {e}")))?
        .build()
        .map_err(|e| ImagerError::BuildError(format!("build ISO ESP spec: {e}")))?;
    let iso_bytes = spawn_blocking(move || {
        let mut out = Cursor::new(Vec::new());
        miso::build_iso(&spec, &mut out)?;
        Ok::<Vec<u8>, MisoError>(out.into_inner())
    })
    .await
    .map_err(|e| ImagerError::BuildError(format!("join ISO build task: {e}")))?
    .map_err(|e| ImagerError::BuildError(format!("build bootable ISO: {e}")))?;

    let iso_path = output_dir.join(Artifact::Iso.filename());
    fs::write(&iso_path, &iso_bytes)
        .await
        .map_err(|e| ImagerError::BuildError(format!("write ISO: {e}")))?;

    Ok(iso_path)
}

/// Builds the raw disk IMG artifact, layering any overlay boot assets.
///
/// # Errors
///
/// Returns an error when reading the UKI, running miso, or writing output fails.
pub async fn build_raw(
    resolved_profile: &ResolvedBuildProfile,
    overlay_assets: &[esp::EspFile],
    output_dir: &Path,
    uki_path: &Path,
) -> Result<PathBuf> {
    let uki_bytes = fs::read(uki_path)
        .await
        .map_err(|e| ImagerError::BuildError(format!("read UKI: {e}")))?;
    let arch = esp_arch(resolved_profile.arch());
    let spec = EspSpecBuilder::default()
        .with_uki(arch, uki_bytes)
        .map_err(|e| ImagerError::BuildError(format!("add UKI to raw ESP spec: {e}")))?
        .add_files(overlay_assets.to_vec())
        .map_err(|e| ImagerError::BuildError(format!("add overlay assets to raw ESP spec: {e}")))?
        .build()
        .map_err(|e| ImagerError::BuildError(format!("build raw ESP spec: {e}")))?;
    let raw_bytes = spawn_blocking(move || {
        let mut out = Cursor::new(Vec::new());
        miso::build_raw(&spec, &mut out, Some(6))?;
        Ok::<Vec<u8>, MisoError>(out.into_inner())
    })
    .await
    .map_err(|e| ImagerError::BuildError(format!("join IMG build task: {e}")))?
    .map_err(|e| ImagerError::BuildError(format!("build raw disk image: {e}")))?;

    let raw_path = output_dir.join(Artifact::Raw.filename());
    fs::write(&raw_path, &raw_bytes)
        .await
        .map_err(|e| ImagerError::BuildError(format!("write IMG: {e}")))?;

    Ok(raw_path)
}

/// Builds the requested artifact from a resolved build profile.
///
/// # Errors
///
/// Returns an error when pulling, staging, or building any artifact fails.
pub async fn build(resolved_profile: &ResolvedBuildProfile, output_dir: &Path) -> Result<()> {
    let work = output_dir.join(".work");
    fs::create_dir_all(&work)
        .await
        .map_err(|e| ImagerError::BuildError(format!("create work dir: {e}")))?;

    let installer_dir = work.join("installer");
    stage::pull_installer(resolved_profile, &installer_dir, None)
        .await
        .map_err(|e| ImagerError::BuildError(format!("pull installer: {e}")))?;

    let assets = stage::load_installer_assets(&installer_dir)?;

    copy_asset_to_output(&assets.kernel, output_dir, Artifact::Kernel).await?;
    copy_asset_to_output(&assets.cmdline, output_dir, Artifact::Cmdline).await?;

    let pulled_dirs = if resolved_profile.extensions().is_empty() {
        vec![]
    } else {
        stage::pull_extensions(
            resolved_profile.extensions(),
            &resolved_profile.arch(),
            &work,
            None,
        )
        .await
        .map_err(|e| ImagerError::BuildError(format!("pull extensions: {e}")))?
    };

    let extra_files: Vec<ramune::ExtraFile<'_>> = pulled_dirs
        .iter()
        .map(|entry| ramune::ExtraFile {
            name: format!("extensions/{}.erofs", extension_archive_name(&entry.0)),
            path: entry.1.as_path(),
            compress: true,
        })
        .collect();

    let initramfs_path = build_merged_initramfs(&assets, &extra_files, output_dir).await?;

    let uki_path = build_uki(&assets, &initramfs_path, output_dir).await?;

    if let Some(overlay) = resolved_profile.overlay() {
        let arch = resolved_profile.arch();
        let overlay_dir = stage::pull_overlay(overlay, &arch, &work, None)
            .await
            .map_err(|e| ImagerError::BuildError(format!("pull overlay: {e}")))?;
        build_iso(resolved_profile, output_dir, &uki_path).await?;
        build_raw(resolved_profile, &overlay_dir, output_dir, &uki_path).await?;
    } else {
        build_iso(resolved_profile, output_dir, &uki_path).await?;
        build_raw(resolved_profile, &[], output_dir, &uki_path).await?;
    }

    Ok(())
}

async fn copy_asset_to_output(source: &Path, output_dir: &Path, artifact: Artifact) -> Result<()> {
    let dest = output_dir.join(artifact.filename());
    if let Some(parent) = dest.parent() {
        fs::create_dir_all(parent)
            .await
            .map_err(|e| ImagerError::BuildError(format!("create dir: {e}")))?;
    }
    fs::copy(source, &dest)
        .await
        .map_err(|e| ImagerError::BuildError(format!("copy {}: {e}", dest.display())))?;
    Ok(())
}

fn esp_arch(arch: Arch) -> EspArch {
    match arch {
        Arch::Amd64 => EspArch::X86_64,
        Arch::Arm64 => EspArch::Aarch64,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::request::Platform;

    fn write_files(dir: &std::path::Path, files: &[&str], data: &[u8]) {
        for name in files {
            std::fs::write(dir.join(name), data).expect("write");
        }
    }

    #[test]
    fn esp_arch_amd64() {
        // ARRANGE
        let arch = Arch::Amd64;

        // ACT
        let result = esp_arch(arch);

        // ASSERT
        assert_eq!(result, esp::Arch::X86_64);
    }

    #[test]
    fn esp_arch_arm64() {
        // ARRANGE
        let arch = Arch::Arm64;

        // ACT
        let result = esp_arch(arch);

        // ASSERT
        assert_eq!(result, esp::Arch::Aarch64);
    }

    #[tokio::test]
    async fn build_merged_initramfs_missing_base() {
        // ARRANGE
        let dir = tempfile::TempDir::new().expect("tempdir");
        let assets = InstallerAssets {
            kernel: dir.path().join("vmlinuz"),
            initramfs: dir.path().join("missing-base.img"),
            stub: dir.path().join("stub.efi"),
            cmdline: dir.path().join("cmdline"),
        };
        let output = dir.path().join("out");

        // ACT
        let result = build_merged_initramfs(&assets, &[], &output).await;

        // ASSERT
        let _err = result.unwrap_err();
    }

    #[tokio::test]
    async fn build_uki_missing_kernel() {
        // ARRANGE
        let dir = tempfile::TempDir::new().expect("tempdir");
        let assets = InstallerAssets {
            kernel: dir.path().join("missing-kernel"),
            initramfs: dir.path().join("initramfs.img"),
            stub: dir.path().join("stub.efi"),
            cmdline: dir.path().join("cmdline"),
        };

        // ACT
        let result = build_uki(&assets, &dir.path().join("initramfs.img"), dir.path()).await;

        // ASSERT
        let _err = result.unwrap_err();
    }

    #[tokio::test]
    async fn build_merges_with_valid_base() {
        // ARRANGE
        let dir = tempfile::TempDir::new().expect("tempdir");

        let rootfs = dir.path().join("rootfs");
        std::fs::create_dir_all(rootfs.join("sbin")).expect("create sbin dir");
        std::fs::write(rootfs.join("sbin").join("init"), b"init").expect("write init");

        let init_file = dir.path().join("init");
        std::fs::write(&init_file, b"#!/bin/sh\nexec /sbin/init\n").expect("write init");

        let base_img = dir.path().join("base.img");
        ramune::create(
            &ramune::CreateConfig {
                init: &init_file,
                rootfs_dir: &rootfs,
                file_contexts: None,
                compression_level: 19,
                rootfs_compression_level: 3,
            },
            &base_img,
        )
        .expect("create base");

        let assets = InstallerAssets {
            kernel: dir.path().join("vmlinuz"),
            initramfs: base_img.clone(),
            stub: dir.path().join("stub.efi"),
            cmdline: dir.path().join("cmdline"),
        };

        let output = dir.path().join("output");
        std::fs::create_dir_all(&output).expect("create output dir");

        // ACT
        let result = build_merged_initramfs(&assets, &[], &output).await;

        // ASSERT
        result.unwrap();
        assert!(output.join("initramfs.img").exists());
    }

    #[tokio::test]
    async fn build_iso_missing_uki() {
        // ARRANGE
        let dir = tempfile::TempDir::new().expect("tempdir");
        let bp = ResolvedBuildProfile::new(
            Platform::Metal,
            "v1.0.0".into(),
            Arch::Amd64,
            vec![],
            None,
            "ghcr.io/installer:v1.0.0".into(),
        );

        // ACT
        let result = build_iso(&bp, dir.path(), &dir.path().join("missing.efi")).await;

        // ASSERT
        let _err = result.unwrap_err();
    }

    #[tokio::test]
    async fn build_raw_missing_uki() {
        // ARRANGE
        let dir = tempfile::TempDir::new().expect("tempdir");
        let bp = ResolvedBuildProfile::new(
            Platform::Metal,
            "v1.0.0".into(),
            Arch::Amd64,
            vec![],
            None,
            "ghcr.io/installer:v1.0.0".into(),
        );

        // ACT
        let result = build_raw(&bp, &[], dir.path(), &dir.path().join("missing.efi")).await;

        // ASSERT
        let _err = result.unwrap_err();
    }

    #[tokio::test]
    async fn build_uki_with_all_inputs() {
        // ARRANGE
        let dir = tempfile::TempDir::new().expect("tempdir");
        write_files(
            dir.path(),
            &["stub.efi", "vmlinuz", "cmdline", "initramfs.img"],
            b"data",
        );
        let output_dir = dir.path().join("out");
        std::fs::create_dir_all(&output_dir).expect("create out");
        let assets = InstallerAssets {
            kernel: dir.path().join("vmlinuz"),
            initramfs: dir.path().join("initramfs.img"),
            stub: dir.path().join("stub.efi"),
            cmdline: dir.path().join("cmdline"),
        };

        // ACT
        let result = build_uki(&assets, &dir.path().join("initramfs.img"), &output_dir).await;

        // ASSERT
        let _err = result.unwrap_err();
    }

    #[tokio::test]
    async fn build_iso_with_uki_file() {
        // ARRANGE
        let dir = tempfile::TempDir::new().expect("tempdir");
        let output_dir = dir.path().join("out");
        std::fs::create_dir_all(&output_dir).expect("create out");
        let uki = dir.path().join("uki.efi");
        std::fs::write(&uki, b"not-a-valid-uki").expect("write uki");
        let bp = ResolvedBuildProfile::new(
            Platform::Metal,
            "v1.0.0".into(),
            Arch::Amd64,
            vec![],
            None,
            "ghcr.io/installer:v1.0.0".into(),
        );

        // ACT
        let result = build_iso(&bp, &output_dir, &uki).await;

        // ASSERT
        let iso_path = result.unwrap();
        assert!(iso_path.exists());
        assert!(iso_path.ends_with("muak.iso"));
    }

    #[tokio::test]
    async fn build_raw_with_uki_file() {
        // ARRANGE
        let dir = tempfile::TempDir::new().expect("tempdir");
        let output_dir = dir.path().join("out");
        std::fs::create_dir_all(&output_dir).expect("create out");
        let uki = dir.path().join("uki.efi");
        std::fs::write(&uki, b"not-a-valid-uki").expect("write uki");
        let bp = ResolvedBuildProfile::new(
            Platform::Metal,
            "v1.0.0".into(),
            Arch::Amd64,
            vec![],
            None,
            "ghcr.io/installer:v1.0.0".into(),
        );

        // ACT
        let result = build_raw(&bp, &[], &output_dir, &uki).await;

        // ASSERT
        let raw_path = result.unwrap();
        assert!(raw_path.exists());
        assert!(raw_path.ends_with("muak.raw.zst"));
    }

    #[tokio::test]
    async fn build_merges_with_extra_files() {
        // ARRANGE
        let dir = tempfile::TempDir::new().expect("tempdir");

        let rootfs = dir.path().join("rootfs");
        std::fs::create_dir_all(rootfs.join("sbin")).expect("create sbin dir");
        std::fs::write(rootfs.join("sbin").join("init"), b"init").expect("write init");

        let init_file = dir.path().join("init");
        std::fs::write(&init_file, b"#!/bin/sh\nexec /sbin/init\n").expect("write init");

        let base_img = dir.path().join("base.img");
        ramune::create(
            &ramune::CreateConfig {
                init: &init_file,
                rootfs_dir: &rootfs,
                file_contexts: None,
                compression_level: 19,
                rootfs_compression_level: 3,
            },
            &base_img,
        )
        .expect("create base");

        let profile = dir.path().join("profile.toml");
        std::fs::write(&profile, b"[customization]\nextensions = []").expect("write profile");

        let extras = [ramune::ExtraFile {
            name: "profile.toml".to_owned(),
            path: &profile,
            compress: false,
        }];

        let assets = InstallerAssets {
            kernel: dir.path().join("vmlinuz"),
            initramfs: base_img.clone(),
            stub: dir.path().join("stub.efi"),
            cmdline: dir.path().join("cmdline"),
        };

        let output = dir.path().join("output");
        std::fs::create_dir_all(&output).expect("create output dir");

        // ACT
        let result = build_merged_initramfs(&assets, &extras, &output).await;

        // ASSERT
        result.unwrap();
        assert!(output.join("initramfs.img").exists());
    }
}
