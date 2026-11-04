//! UKI build and EFI deployment during installation.

use std::fs::File;
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};

use anyhow::{Context as _, Result};
use config::system::host;
use luks2::key::Passphrase;
use sbolt::keys::SigningPair;
use sbolt::keys::hierarchy::Bundle;
use uki::measure::MeasuredSection;
use wizard::artifact::Artifact;
use wizard::domain::profile::{CustomizationSpec, Profile};
use wizard::request::Request;

use crate::efi;

/// Deploys the UKI to the EFI partition on a blocking thread.
pub(super) async fn spawn(
    efi_part: String,
    host: host::Config,
    booted_profile: Profile,
    luks_key: Passphrase,
    tpm_available: bool,
    sb_hierarchy: Option<Bundle>,
) -> Result<(Vec<MeasuredSection>, Option<Bundle>)> {
    tokio::task::spawn_blocking(move || {
        run(
            &efi_part,
            &host,
            &booted_profile,
            &luks_key,
            tpm_available,
            sb_hierarchy,
        )
    })
    .await
    .context("EFI deployment task")?
}

fn run(
    efi_part: &str,
    host: &host::Config,
    booted_profile: &Profile,
    luks_key: &[u8],
    tpm_available: bool,
    sb_hierarchy: Option<Bundle>,
) -> Result<(Vec<MeasuredSection>, Option<Bundle>)> {
    let install_profile = derive_install_profile(booted_profile, &host.extensions)?;

    efi::mount(efi_part)?;
    let deployed = deploy(
        &install_profile,
        host,
        luks_key,
        tpm_available,
        sb_hierarchy,
    );
    efi::unmount();

    deployed
}

fn deploy(
    install_profile: &Profile,
    host: &host::Config,
    luks_key: &[u8],
    tpm_available: bool,
    sb_hierarchy: Option<Bundle>,
) -> Result<(Vec<MeasuredSection>, Option<Bundle>)> {
    let esp_root = PathBuf::from(efi::MOUNT_POINT);
    let mut uki_file = efi::create(&esp_root, esp::arch::Arch::current().boot_path())?;

    let (overlay_r, mut overlay_w) = if install_profile.overlay().is_some() {
        let (reader, writer) = UnixStream::pair()?;
        (Some(reader), Some(writer))
    } else {
        (None, None)
    };

    let (metadata, sb_hierarchy) = std::thread::scope(|scope| -> Result<_> {
        let demux = overlay_r
            .map(move |mut reader| scope.spawn(move || efi::extract_tar(&esp_root, &mut reader)));

        let built = build(
            &mut uki_file,
            install_profile,
            host,
            overlay_w.as_mut(),
            sb_hierarchy,
        );

        drop(overlay_w);

        if let Some(demux) = demux {
            demux
                .join()
                .map_err(|_error| anyhow::anyhow!("overlay extraction thread panicked"))??;
        }

        built
    })?;

    if !tpm_available {
        efi::write_bytes(Path::new(efi::MOUNT_POINT), "luks", luks_key)?;
    }

    Ok((metadata.sections, sb_hierarchy))
}

fn build(
    uki_file: &mut File,
    install_profile: &Profile,
    host: &host::Config,
    overlay_w: Option<&mut UnixStream>,
    sb_hierarchy: Option<Bundle>,
) -> Result<(wizard::metadata::Metadata, Option<Bundle>)> {
    let pair = sb_hierarchy.as_ref().map(|hierarchy| SigningPair {
        signer: &hierarchy.db.signer,
        certificate: &hierarchy.db.certificate,
    });

    let mut request =
        Request::new(host.version.clone(), &host.registry).artifact(Artifact::Uki, uki_file)?;
    if let Some(writer) = overlay_w {
        request = request.artifact(Artifact::Overlays, writer)?;
    }

    let request = match pair.as_ref() {
        Some(pair) => request.sign(pair),
        None => request,
    };

    let metadata = request
        .build(install_profile)
        .context("wizard install build")?;

    Ok((metadata, sb_hierarchy))
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
