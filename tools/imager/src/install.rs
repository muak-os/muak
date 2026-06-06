//! Public install-asset preparation API.

use std::path::{Path, PathBuf};

use esp::EspFile;
use tokio::fs;

use crate::error::{ImagerError, Result};
use crate::profile::{self, Profile};
use crate::render;
use crate::request::Request;
use crate::resolve::{self, Config, ResolvedProfile};
use crate::stage;
use crate::workspace;

/// Prepared install assets produced from a profile.
#[derive(Debug, Clone)]
pub struct Assets {
    kernel: PathBuf,
    initramfs: PathBuf,
    cmdline: PathBuf,
    stub: PathBuf,
    uki: Option<PathBuf>,
    esp_files: Vec<EspFile>,
    profile_id: profile::Id,
    resolved_profile: ResolvedProfile,
}

impl Assets {
    /// Returns the path to the kernel binary.
    #[must_use]
    pub fn kernel(&self) -> &Path {
        &self.kernel
    }

    /// Returns the path to the merged initramfs.
    #[must_use]
    pub fn initramfs(&self) -> &Path {
        &self.initramfs
    }

    /// Returns the path to the kernel command-line file.
    #[must_use]
    pub fn cmdline(&self) -> &Path {
        &self.cmdline
    }

    /// Returns the path to the UEFI stub.
    #[must_use]
    pub fn stub(&self) -> &Path {
        &self.stub
    }

    /// Returns the path to the generic UKI, if built.
    #[must_use]
    pub fn uki(&self) -> Option<&Path> {
        self.uki.as_deref()
    }

    /// Returns the overlay ESP file set.
    #[must_use]
    pub fn esp_files(&self) -> &[EspFile] {
        &self.esp_files
    }

    /// Returns the profile identity.
    #[must_use]
    pub fn profile_id(&self) -> &profile::Id {
        &self.profile_id
    }

    /// Returns the resolved build profile.
    #[must_use]
    pub fn resolved_profile(&self) -> &ResolvedProfile {
        &self.resolved_profile
    }
}

/// Prepares install assets from a profile for local machine deployment.
///
/// # Errors
///
/// Returns an error when resolution, pulling, or building fails.
pub async fn assets(
    request: &Request,
    profile: &Profile,
    config: &Config,
    output_dir: &Path,
) -> Result<Assets> {
    let resolved = resolve::profile(request, profile, &config.sources)?;
    let profile_bytes = profile.canonical_bytes()?;
    let profile_id = profile.id()?;
    let workspace = workspace::unique(&config.workspace_root);

    fs::create_dir_all(output_dir)
        .await
        .map_err(|e| ImagerError::BuildError(format!("create output dir: {e}")))?;
    fs::create_dir_all(&workspace)
        .await
        .map_err(|e| ImagerError::BuildError(format!("create workspace: {e}")))?;

    let prepared = render::prepare(&resolved, &profile_bytes, &workspace, output_dir).await?;

    let kernel = copy(&prepared.assets.kernel, output_dir, "kernel").await?;
    let cmdline = copy(&prepared.assets.cmdline, output_dir, "cmdline").await?;
    let stub = copy(&prepared.assets.stub, output_dir, "stub.efi").await?;

    let esp_files = if let Some(overlay) = resolved.overlay() {
        stage::pull_overlay(overlay, &resolved.arch(), &workspace, None)
            .await
            .map_err(|e| ImagerError::BuildError(format!("pull overlay: {e}")))?
    } else {
        vec![]
    };

    Ok(Assets {
        kernel,
        initramfs: prepared.initramfs,
        cmdline,
        stub,
        uki: Some(prepared.uki),
        esp_files,
        profile_id,
        resolved_profile: resolved,
    })
}

async fn copy(source: &Path, output_dir: &Path, name: &str) -> Result<PathBuf> {
    let dest = output_dir.join(name);
    fs::copy(source, &dest)
        .await
        .map_err(|e| ImagerError::BuildError(format!("copy {name}: {e}")))?;
    Ok(dest)
}

#[cfg(test)]
mod tests {
    use koci::arch::Arch;

    use super::*;
    use crate::request::Platform;

    fn empty_profile() -> Profile {
        Profile::from_toml(b"[customization]\nextensions = []").expect("parse")
    }

    #[test]
    fn assets_accessors() {
        // ARRANGE
        let dir = tempfile::TempDir::new().expect("tempdir");
        let resolved = ResolvedProfile::new(
            Platform::Metal,
            "v1.0.0".into(),
            Arch::Amd64,
            vec![],
            None,
            "ghcr.io/installer:v1.0.0".into(),
        );
        let pid = empty_profile().id().expect("id");
        let kernel = dir.path().join("kernel");
        let initramfs = dir.path().join("initramfs.img");
        let cmdline = dir.path().join("cmdline");
        let stub = dir.path().join("stub.efi");
        let uki = dir.path().join("uki.efi");

        let assets = Assets {
            kernel: kernel.clone(),
            initramfs: initramfs.clone(),
            cmdline: cmdline.clone(),
            stub: stub.clone(),
            uki: Some(uki.clone()),
            esp_files: vec![EspFile {
                path: "dtb/rpi.dtb".into(),
                data: b"dtb".to_vec(),
            }],
            profile_id: pid.clone(),
            resolved_profile: resolved,
        };

        // ACT / ASSERT
        assert_eq!(assets.kernel(), kernel);
        assert_eq!(assets.initramfs(), initramfs);
        assert_eq!(assets.cmdline(), cmdline);
        assert_eq!(assets.stub(), stub);
        assert_eq!(assets.uki(), Some(uki.as_path()));
        assert_eq!(assets.esp_files().len(), 1);
        assert_eq!(
            assets.esp_files().first().expect("first esp file").path,
            "dtb/rpi.dtb"
        );
        assert_eq!(assets.profile_id(), &pid);
    }

    #[test]
    fn assets_without_uki() {
        // ARRANGE
        let dir = tempfile::TempDir::new().expect("tempdir");
        let resolved = ResolvedProfile::new(
            Platform::Metal,
            "v1.0.0".into(),
            Arch::Amd64,
            vec![],
            None,
            "ghcr.io/installer:v1.0.0".into(),
        );
        let pid = empty_profile().id().expect("id");

        let assets = Assets {
            kernel: dir.path().join("kernel"),
            initramfs: dir.path().join("initramfs.img"),
            cmdline: dir.path().join("cmdline"),
            stub: dir.path().join("stub.efi"),
            uki: None,
            esp_files: vec![],
            profile_id: pid,
            resolved_profile: resolved,
        };

        // ACT / ASSERT
        assert!(assets.uki().is_none());
        assert!(assets.esp_files().is_empty());
    }

    #[test]
    fn different_extensions_produce_different_profile_ids() {
        // ARRANGE
        let first =
            Profile::from_toml(b"[customization]\nextensions = [\"muak-os/qemu\"]").expect("parse");
        let second = Profile::from_toml(b"[customization]\nextensions = []").expect("parse");

        // ACT
        let id_first = first.id().expect("id");
        let id_second = second.id().expect("id");

        // ASSERT
        assert_ne!(id_first, id_second);
    }

    #[test]
    fn identical_overlay_different_extensions_produce_different_ids() {
        // ARRANGE
        let first = Profile::from_toml(
            b"[overlay]\nname = \"rpi\"\nimage = \"sbc\"\n[customization]\nextensions = [\"muak-os/qemu\"]",
        )
        .expect("parse");
        let second = Profile::from_toml(
            b"[overlay]\nname = \"rpi\"\nimage = \"sbc\"\n[customization]\nextensions = []",
        )
        .expect("parse");

        // ACT
        let id_first = first.id().expect("id");
        let id_second = second.id().expect("id");

        // ASSERT
        assert_ne!(id_first, id_second);
    }
}
