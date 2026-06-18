//! Build pipeline orchestration.

use std::collections::HashMap;
use std::io::Cursor;
use std::io::Read as _;
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};

use sbolt::keys::SigningPair;
use sbolt::signature;
use tokio::fs;
use tokio::task::spawn_blocking;
use yuki::BuildInput;
use yuki::SizedPart;
use yuki::section::Section;

use super::archive;
use super::media;
use super::stage::{self, InstallerAssets};
use super::uki;
use crate::artifact::Artifact;
use crate::error::{ImagerError, Result};
use crate::resolve::ResolvedProfile;

/// Prebuilt intermediate artifacts shared across rendering paths.
pub(crate) struct Prepared {
    pub assets: InstallerAssets,
    pub initramfs_tail: Vec<u8>,
    pub uki_bytes: Vec<u8>,
    pub sections: Vec<Section>,
}

/// Pulls the installer, builds the initramfs tail, and builds a UKI.
pub(crate) async fn prepare(
    resolved_profile: &ResolvedProfile,
    profile_bytes: &[u8],
    signing_key: Option<&SigningPair<'_>>,
) -> Result<Prepared> {
    let installer = stage::pull_installer(resolved_profile, None).await?;
    let assets = stage::load_installer_assets(&installer)?;
    let initramfs_tail = archive::build_initramfs_tail(resolved_profile, profile_bytes).await?;

    let stub = stage::read_file(&assets.stub, "stub")?;
    let kernel = stage::read_file(&assets.kernel, "kernel")?;
    let cmdline = stage::read_file(&assets.cmdline, "cmdline")?;
    let base_file = assets.initramfs.clone();
    let base_len = base_file.len;
    let tail = initramfs_tail.clone();
    let initramfs_len = base_len.saturating_add(u64::try_from(tail.len()).unwrap_or(u64::MAX));
    let stub_len = u64::try_from(stub.len()).unwrap_or(u64::MAX);
    let kernel_len = u64::try_from(kernel.len()).unwrap_or(u64::MAX);
    let cmdline_len = u64::try_from(cmdline.len()).unwrap_or(u64::MAX);

    let (writer, mut reader) =
        UnixStream::pair().map_err(|e| ImagerError::BuildError(format!("create pipe: {e}")))?;

    let sections_handle = spawn_blocking(move || {
        let mut stub_reader = Cursor::new(stub);
        let mut kernel_reader = Cursor::new(kernel);
        let mut cmdline_reader = Cursor::new(cmdline);
        let base_reader = base_file
            .open()
            .map_err(|e| ImagerError::BuildError(format!("open initramfs: {e}")))?;
        let tail_reader = Cursor::new(tail.as_slice());
        let mut initramfs_reader = base_reader.chain(tail_reader);

        let input = BuildInput {
            stub: SizedPart {
                len: stub_len,
                reader: &mut stub_reader,
            },
            kernel: SizedPart {
                len: kernel_len,
                reader: &mut kernel_reader,
            },
            initramfs: SizedPart {
                len: initramfs_len,
                reader: &mut initramfs_reader,
            },
            cmdline: SizedPart {
                len: cmdline_len,
                reader: &mut cmdline_reader,
            },
            dtb: None,
        };
        yuki::build(input, &mut &writer)
            .map_err(|e| ImagerError::BuildError(format!("build UKI: {e}")))
    });

    let uki_bytes = if let Some(key) = signing_key {
        let mut signed = Vec::new();
        signature::sign(&mut reader, key.signer, key.certificate, &mut signed)
            .map_err(|e| ImagerError::BuildError(format!("sign UKI: {e}")))?;
        signed
    } else {
        let mut buf = Vec::new();
        std::io::copy(&mut reader, &mut buf)
            .map_err(|e| ImagerError::BuildError(format!("read UKI pipe: {e}")))?;
        buf
    };

    let sections = sections_handle
        .await
        .map_err(|e| ImagerError::BuildError(format!("join UKI build task: {e}")))??;

    Ok(Prepared {
        assets,
        initramfs_tail,
        uki_bytes,
        sections,
    })
}

/// Builds the requested artifacts sharing a single resolution.
///
/// # Errors
///
/// Returns an error when pulling, staging, building, or signing fails.
pub async fn artifacts(
    resolved_profile: &ResolvedProfile,
    requested: &[Artifact],
    signing_key: Option<&SigningPair<'_>>,
    profile_bytes: &[u8],
    output_dir: &Path,
) -> Result<HashMap<Artifact, PathBuf>> {
    let prepared = prepare(resolved_profile, profile_bytes, signing_key).await?;
    let uki_bytes = &prepared.uki_bytes;

    let mut results = HashMap::new();

    if requested.contains(&Artifact::Kernel) {
        let kernel_path = output_dir.join(Artifact::Kernel.filename());
        fs::write(
            &kernel_path,
            stage::read_file(&prepared.assets.kernel, "kernel")?,
        )
        .await
        .map_err(|e| ImagerError::BuildError(format!("write kernel: {e}")))?;
        results.insert(Artifact::Kernel, kernel_path);
    }
    if requested.contains(&Artifact::Cmdline) {
        let cmdline_path = output_dir.join(Artifact::Cmdline.filename());
        fs::write(
            &cmdline_path,
            stage::read_file(&prepared.assets.cmdline, "cmdline")?,
        )
        .await
        .map_err(|e| ImagerError::BuildError(format!("write cmdline: {e}")))?;
        results.insert(Artifact::Cmdline, cmdline_path);
    }
    if requested.contains(&Artifact::Initramfs) {
        let initramfs_path = output_dir.join(Artifact::Initramfs.filename());
        uki::write_initramfs(&prepared.assets, &prepared.initramfs_tail, &initramfs_path).await?;
        results.insert(Artifact::Initramfs, initramfs_path);
    }
    if requested.contains(&Artifact::Uki) {
        let uki_path = output_dir.join(Artifact::Uki.filename());
        fs::write(&uki_path, &uki_bytes)
            .await
            .map_err(|e| ImagerError::BuildError(format!("write UKI: {e}")))?;
        results.insert(Artifact::Uki, uki_path);
    }
    if requested.contains(&Artifact::Iso) {
        results.insert(
            Artifact::Iso,
            media::iso(resolved_profile, output_dir, uki_bytes).await?,
        );
    }

    let needs_overlay = requested.contains(&Artifact::Raw) || requested.contains(&Artifact::Esp);
    let overlay_files = if needs_overlay {
        pull_overlay_if_present(resolved_profile).await?
    } else {
        vec![]
    };

    if requested.contains(&Artifact::Raw) {
        results.insert(
            Artifact::Raw,
            media::raw(resolved_profile, &overlay_files, output_dir, uki_bytes).await?,
        );
    }
    if requested.contains(&Artifact::Esp) {
        let esp_dir = output_dir.join(Artifact::Esp.filename());
        media::write_esp_files(&overlay_files, &esp_dir).await?;
        results.insert(Artifact::Esp, esp_dir);
    }

    Ok(results)
}

pub(crate) async fn pull_overlay_if_present(
    resolved_profile: &ResolvedProfile,
) -> Result<Vec<esp::EspFile>> {
    if let Some(overlay) = resolved_profile.overlay() {
        stage::pull_overlay(overlay, &resolved_profile.arch(), None)
            .await
            .map_err(|e| ImagerError::BuildError(format!("pull overlay: {e}")))
    } else {
        Ok(vec![])
    }
}
