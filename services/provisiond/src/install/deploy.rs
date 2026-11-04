//! UKI build and EFI deployment during installation.

use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};

use anyhow::{Context as _, Result};
use config::system;
use sbolt::keys::SigningPair;
use sbolt::keys::hierarchy::Bundle;
use tokio::sync::mpsc;
use wizard::artifact::Artifact;
use wizard::domain::profile::{CustomizationSpec, Profile};
use wizard::request::Request;

use super::send_progress;
use crate::efi;
use crate::ipc::proto::provision::InstallProgress;

pub(super) async fn build_and_deploy_efi(
    efi_part: &str,
    config: &system::Config,
    booted_profile: Profile,
    luks_key: &[u8],
    tpm_available: bool,
    sb_hierarchy: Option<Bundle>,
    progress: &mpsc::Sender<InstallProgress>,
) -> Result<(Vec<uki::measure::MeasuredSection>, Option<Bundle>)> {
    send_progress(progress, "Building and deploying EFI").await;

    let install_profile = derive_install_profile(&booted_profile, &config.host.extensions)?;
    let has_overlay = install_profile.overlay().is_some();

    let registry = config.host.registry.clone();
    let version = config.host.version.clone();

    efi::mount(efi_part)?;

    let mut uki_file = efi::create(
        Path::new(efi::MOUNT_POINT),
        esp::arch::Arch::current().boot_path(),
    )?;

    let (mut overlay_r, mut overlay_w) = if has_overlay {
        let (reader, writer) = UnixStream::pair()?;
        (Some(reader), Some(writer))
    } else {
        (None, None)
    };

    let esp_root = PathBuf::from(efi::MOUNT_POINT);
    let demux = overlay_r.take().map(|mut reader| {
        tokio::task::spawn_blocking(move || efi::extract_tar(&esp_root, &mut reader))
    });

    let (metadata, sb_hierarchy) = tokio::task::spawn_blocking(move || {
        let pair = sb_hierarchy.as_ref().map(|hierarchy| SigningPair {
            signer: &hierarchy.db.signer,
            certificate: &hierarchy.db.certificate,
        });

        let mut request =
            Request::new(version, &registry).artifact(Artifact::Uki, &mut uki_file)?;
        if let Some(writer) = overlay_w.as_mut() {
            request = request.artifact(Artifact::Overlays, writer)?;
        }

        let request = match pair.as_ref() {
            Some(pair) => request.sign(pair),
            None => request,
        };

        let metadata = request
            .build(&install_profile)
            .context("wizard install build")?;

        Ok::<_, anyhow::Error>((metadata, sb_hierarchy))
    })
    .await
    .context("wizard install task")??;

    if !tpm_available {
        efi::write_bytes(Path::new(efi::MOUNT_POINT), "luks", luks_key)?;
    }

    if let Some(demux) = demux {
        demux.await??;
    }

    efi::unmount();

    Ok((metadata.sections, sb_hierarchy))
}

fn derive_install_profile(booted: &Profile, extensions: &[String]) -> Result<Profile> {
    let customization =
        CustomizationSpec::new(extensions.to_vec()).context("invalid extensions")?;

    Ok(Profile::new(
        booted.overlay().cloned(),
        customization,
        booted.kernel().clone(),
    ))
}
