//! Artifact rendering functions for staged installer inputs.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use esp::Arch as EspArch;
use esp::EspSpecBuilder;
use koci::arch::Arch;
use tokio::fs;
use tokio::task::spawn_blocking;

use crate::artifact::Artifact;
use crate::catalog::extension_archive_name;
use crate::error::{ImagerError, Result};
use crate::resolve::ResolvedProfile;
use crate::stage::{self, InstallerAssets};

/// Pre-built intermediate artifacts shared across rendering paths.
pub(crate) struct Prepared {
    /// Loaded installer assets.
    pub assets: InstallerAssets,
    /// Path to the merged initramfs.
    pub initramfs: PathBuf,
    /// Path to the generic UKI.
    pub uki: PathBuf,
}

/// Pulls the installer, builds the initramfs (with extensions and profile), and builds a UKI.
///
/// # Errors
///
/// Returns an error when pulling, staging, or building fails.
pub(crate) async fn prepare(
    resolved_profile: &ResolvedProfile,
    profile_bytes: &[u8],
    workspace: &Path,
    output_dir: &Path,
) -> Result<Prepared> {
    let installer_dir = workspace.join("installer");
    stage::pull_installer(resolved_profile, &installer_dir, None)
        .await
        .map_err(|e| ImagerError::BuildError(format!("pull installer: {e}")))?;

    let assets = stage::load_installer_assets(&installer_dir)?;
    let initramfs = build_initramfs(
        resolved_profile,
        &assets,
        profile_bytes,
        workspace,
        output_dir,
    )
    .await?;
    let uki = uki(&assets, &initramfs, output_dir).await?;

    Ok(Prepared {
        assets,
        initramfs,
        uki,
    })
}

/// Builds the requested artifacts sharing a single resolution and workspace.
///
/// # Errors
///
/// Returns an error when pulling, staging, or building fails.
pub async fn artifacts(
    resolved_profile: &ResolvedProfile,
    requested: &[Artifact],
    profile_bytes: &[u8],
    output_dir: &Path,
    workspace: &Path,
) -> Result<HashMap<Artifact, PathBuf>> {
    fs::create_dir_all(workspace)
        .await
        .map_err(|e| ImagerError::BuildError(format!("create workspace: {e}")))?;

    let prepared = prepare(resolved_profile, profile_bytes, workspace, output_dir).await?;
    let mut results = HashMap::new();

    if requested.contains(&Artifact::Kernel) {
        results.insert(
            Artifact::Kernel,
            copy_to_output(&prepared.assets.kernel, output_dir, Artifact::Kernel).await?,
        );
    }
    if requested.contains(&Artifact::Cmdline) {
        results.insert(
            Artifact::Cmdline,
            copy_to_output(&prepared.assets.cmdline, output_dir, Artifact::Cmdline).await?,
        );
    }
    if requested.contains(&Artifact::Initramfs) {
        results.insert(Artifact::Initramfs, prepared.initramfs.clone());
    }
    if requested.contains(&Artifact::Uki) {
        results.insert(Artifact::Uki, prepared.uki.clone());
    }
    if requested.contains(&Artifact::Iso) {
        results.insert(
            Artifact::Iso,
            iso(resolved_profile, output_dir, &prepared.uki).await?,
        );
    }

    let needs_overlay = requested.contains(&Artifact::Raw) || requested.contains(&Artifact::Esp);
    let overlay_files = if needs_overlay {
        pull_overlay_if_present(resolved_profile, workspace).await?
    } else {
        vec![]
    };

    if requested.contains(&Artifact::Raw) {
        results.insert(
            Artifact::Raw,
            raw(resolved_profile, &overlay_files, output_dir, &prepared.uki).await?,
        );
    }
    if requested.contains(&Artifact::Esp) {
        let esp_dir = output_dir.join(Artifact::Esp.filename());
        write_esp_files(&overlay_files, &esp_dir).await?;
        results.insert(Artifact::Esp, esp_dir);
    }

    Ok(results)
}

async fn copy_to_output(source: &Path, output_dir: &Path, artifact: Artifact) -> Result<PathBuf> {
    let dest = output_dir.join(artifact.filename());
    if let Some(parent) = dest.parent() {
        fs::create_dir_all(parent)
            .await
            .map_err(|e| ImagerError::BuildError(format!("create dir: {e}")))?;
    }
    fs::copy(source, &dest)
        .await
        .map_err(|e| ImagerError::BuildError(format!("copy {}: {e}", dest.display())))?;
    Ok(dest)
}

async fn build_initramfs(
    resolved_profile: &ResolvedProfile,
    assets: &InstallerAssets,
    profile_bytes: &[u8],
    workspace: &Path,
    output_dir: &Path,
) -> Result<PathBuf> {
    let pulled = pull_extensions(resolved_profile, workspace).await?;
    let mut extra_files: Vec<ramune::ExtraFile<'_>> = pulled
        .iter()
        .map(|entry| ramune::ExtraFile {
            name: format!("extensions/{}.erofs", extension_archive_name(&entry.0)),
            path: entry.1.as_path(),
            compress: true,
        })
        .collect();

    let profile_file = embed_profile(profile_bytes, workspace).await?;
    if let Some(ref path) = profile_file {
        extra_files.push(ramune::ExtraFile {
            name: "profile.toml".to_owned(),
            path,
            compress: false,
        });
    }

    merged_initramfs(assets, &extra_files, output_dir).await
}

async fn pull_extensions(
    resolved_profile: &ResolvedProfile,
    workspace: &Path,
) -> Result<Vec<(String, PathBuf)>> {
    let resolved_extensions = resolved_profile.extensions();
    if resolved_extensions.is_empty() {
        return Ok(vec![]);
    }

    stage::pull_extensions(
        resolved_extensions,
        &resolved_profile.arch(),
        workspace,
        None,
    )
    .await
    .map_err(|e| ImagerError::BuildError(format!("pull extensions: {e}")))
}

async fn embed_profile(profile_bytes: &[u8], workspace: &Path) -> Result<Option<PathBuf>> {
    if profile_bytes.is_empty() {
        return Ok(None);
    }
    let profile_path = workspace.join("profile.toml");
    fs::write(&profile_path, profile_bytes)
        .await
        .map_err(|e| ImagerError::BuildError(format!("write profile: {e}")))?;
    Ok(Some(profile_path))
}

async fn pull_overlay_if_present(
    resolved_profile: &ResolvedProfile,
    workspace: &Path,
) -> Result<Vec<esp::EspFile>> {
    if let Some(overlay) = resolved_profile.overlay() {
        stage::pull_overlay(overlay, &resolved_profile.arch(), workspace, None)
            .await
            .map_err(|e| ImagerError::BuildError(format!("pull overlay: {e}")))
    } else {
        Ok(vec![])
    }
}

async fn merged_initramfs(
    assets: &InstallerAssets,
    extra_files: &[ramune::ExtraFile<'_>],
    output_dir: &Path,
) -> Result<PathBuf> {
    let output_path = output_dir.join(Artifact::Initramfs.filename());
    let config = ramune::ExtendConfig {
        base: &assets.initramfs,
        extra_files,
        compression_level: ramune::DEFAULT_ZSTD_COMPRESSION_LEVEL,
    };

    ramune::extend(&config, &output_path)
        .await
        .map_err(|e| ImagerError::BuildError(format!("build initramfs: {e}")))?;

    Ok(output_path)
}

async fn uki(
    assets: &InstallerAssets,
    initramfs_path: &Path,
    output_dir: &Path,
) -> Result<PathBuf> {
    let output_path = output_dir.join(Artifact::Uki.filename());
    let stub = fs::read(&assets.stub)
        .await
        .map_err(|e| ImagerError::BuildError(format!("read stub: {e}")))?;
    let kernel = fs::read(&assets.kernel)
        .await
        .map_err(|e| ImagerError::BuildError(format!("read kernel: {e}")))?;
    let initramfs = fs::read(initramfs_path)
        .await
        .map_err(|e| ImagerError::BuildError(format!("read initramfs: {e}")))?;
    let cmdline = fs::read(&assets.cmdline)
        .await
        .map_err(|e| ImagerError::BuildError(format!("read cmdline: {e}")))?;

    let uki_bytes = spawn_blocking(move || {
        yuki::build(&yuki::BuildInput {
            stub: &stub,
            kernel: &kernel,
            initramfs: &initramfs,
            cmdline: &cmdline,
            dtb: None,
        })
        .map_err(|e| ImagerError::BuildError(format!("build UKI: {e}")))
    })
    .await
    .map_err(|e| ImagerError::BuildError(format!("join UKI build task: {e}")))??;

    fs::write(&output_path, uki_bytes)
        .await
        .map_err(|e| ImagerError::BuildError(format!("write UKI: {e}")))?;

    Ok(output_path)
}

async fn iso(
    resolved_profile: &ResolvedProfile,
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

    let iso_path = output_dir.join(Artifact::Iso.filename());
    let iso_path_clone = iso_path.clone();
    spawn_blocking(move || {
        let file = std::fs::File::create(&iso_path_clone).map_err(std::io::Error::other)?;
        let mut writer = std::io::BufWriter::new(file);
        miso::build_iso(&spec, &mut writer).map_err(std::io::Error::other)
    })
    .await
    .map_err(|e| ImagerError::BuildError(format!("join ISO build task: {e}")))?
    .map_err(|e| ImagerError::BuildError(format!("build bootable ISO: {e}")))?;

    Ok(iso_path)
}

async fn raw(
    resolved_profile: &ResolvedProfile,
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

    let raw_path = output_dir.join(Artifact::Raw.filename());
    let raw_path_clone = raw_path.clone();
    spawn_blocking(move || {
        let file = std::fs::File::create(&raw_path_clone).map_err(std::io::Error::other)?;
        let mut writer = std::io::BufWriter::new(file);
        miso::build_raw(&spec, &mut writer, Some(6)).map_err(std::io::Error::other)
    })
    .await
    .map_err(|e| ImagerError::BuildError(format!("join IMG build task: {e}")))?
    .map_err(|e| ImagerError::BuildError(format!("build raw disk image: {e}")))?;

    Ok(raw_path)
}

async fn write_esp_files(files: &[esp::EspFile], esp_dir: &Path) -> Result<()> {
    for file in files {
        let dest = esp_dir.join(&file.path);
        if let Some(parent) = dest.parent() {
            fs::create_dir_all(parent)
                .await
                .map_err(|e| ImagerError::BuildError(format!("create esp dir: {e}")))?;
        }
        fs::write(&dest, &file.data)
            .await
            .map_err(|e| ImagerError::BuildError(format!("write esp file: {e}")))?;
    }
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
    async fn merged_initramfs_missing_base() {
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
        let result = merged_initramfs(&assets, &[], &output).await;

        // ASSERT
        let _err = result.unwrap_err();
    }

    #[tokio::test]
    async fn merged_initramfs_with_valid_base() {
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
        let result = merged_initramfs(&assets, &[], &output).await;

        // ASSERT
        result.unwrap();
        assert!(output.join("initramfs.img").exists());
    }

    #[tokio::test]
    async fn merged_initramfs_with_extra_files() {
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
        let result = merged_initramfs(&assets, &extras, &output).await;

        // ASSERT
        result.unwrap();
        assert!(output.join("initramfs.img").exists());
    }

    #[tokio::test]
    async fn uki_missing_kernel() {
        // ARRANGE
        let dir = tempfile::TempDir::new().expect("tempdir");
        let assets = InstallerAssets {
            kernel: dir.path().join("missing-kernel"),
            initramfs: dir.path().join("initramfs.img"),
            stub: dir.path().join("stub.efi"),
            cmdline: dir.path().join("cmdline"),
        };

        // ACT
        let result = uki(&assets, &dir.path().join("initramfs.img"), dir.path()).await;

        // ASSERT
        let _err = result.unwrap_err();
    }

    #[tokio::test]
    async fn uki_with_all_inputs() {
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
        let result = uki(&assets, &dir.path().join("initramfs.img"), &output_dir).await;

        // ASSERT
        let _err = result.unwrap_err();
    }

    #[tokio::test]
    async fn iso_missing_uki() {
        // ARRANGE
        let dir = tempfile::TempDir::new().expect("tempdir");
        let bp = ResolvedProfile::new(
            Platform::Metal,
            "v1.0.0".into(),
            Arch::Amd64,
            vec![],
            None,
            "ghcr.io/installer:v1.0.0".into(),
        );

        // ACT
        let result = iso(&bp, dir.path(), &dir.path().join("missing.efi")).await;

        // ASSERT
        let _err = result.unwrap_err();
    }

    #[tokio::test]
    async fn iso_with_uki_file() {
        // ARRANGE
        let dir = tempfile::TempDir::new().expect("tempdir");
        let output_dir = dir.path().join("out");
        std::fs::create_dir_all(&output_dir).expect("create out");
        let uki = dir.path().join("uki.efi");
        std::fs::write(&uki, b"not-a-valid-uki").expect("write uki");
        let bp = ResolvedProfile::new(
            Platform::Metal,
            "v1.0.0".into(),
            Arch::Amd64,
            vec![],
            None,
            "ghcr.io/installer:v1.0.0".into(),
        );

        // ACT
        let result = iso(&bp, &output_dir, &uki).await;

        // ASSERT
        let iso_path = result.unwrap();
        assert!(iso_path.exists());
        assert!(iso_path.ends_with("muak.iso"));
    }

    #[tokio::test]
    async fn raw_missing_uki() {
        // ARRANGE
        let dir = tempfile::TempDir::new().expect("tempdir");
        let bp = ResolvedProfile::new(
            Platform::Metal,
            "v1.0.0".into(),
            Arch::Amd64,
            vec![],
            None,
            "ghcr.io/installer:v1.0.0".into(),
        );

        // ACT
        let result = raw(&bp, &[], dir.path(), &dir.path().join("missing.efi")).await;

        // ASSERT
        let _err = result.unwrap_err();
    }

    #[tokio::test]
    async fn raw_with_uki_file() {
        // ARRANGE
        let dir = tempfile::TempDir::new().expect("tempdir");
        let output_dir = dir.path().join("out");
        std::fs::create_dir_all(&output_dir).expect("create out");
        let uki = dir.path().join("uki.efi");
        std::fs::write(&uki, b"not-a-valid-uki").expect("write uki");
        let bp = ResolvedProfile::new(
            Platform::Metal,
            "v1.0.0".into(),
            Arch::Amd64,
            vec![],
            None,
            "ghcr.io/installer:v1.0.0".into(),
        );

        // ACT
        let result = raw(&bp, &[], &output_dir, &uki).await;

        // ASSERT
        let raw_path = result.unwrap();
        assert!(raw_path.exists());
        assert!(raw_path.ends_with("muak.raw.zst"));
    }

    #[tokio::test]
    async fn embed_profile_writes_file() {
        // ARRANGE
        let dir = tempfile::TempDir::new().expect("tempdir");
        let profile_bytes = b"[customization]\nextensions = [\"muak-os/qemu\"]";

        // ACT
        let result = embed_profile(profile_bytes, dir.path()).await;

        // ASSERT
        let path = result.expect("embed").expect("some");
        assert!(path.exists());
        let contents = std::fs::read_to_string(&path).expect("read");
        assert!(contents.contains("muak-os/qemu"));
    }

    #[tokio::test]
    async fn embed_profile_skips_empty_bytes() {
        // ARRANGE
        let dir = tempfile::TempDir::new().expect("tempdir");

        // ACT
        let result = embed_profile(&[], dir.path()).await;

        // ASSERT
        assert!(result.expect("embed").is_none());
    }
}
