//! Installation workflow orchestration.

mod layout;
pub mod pki;
pub(crate) mod plan;
mod state;
mod volumes;

use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};

use anyhow::{Context as _, Result, bail};
use config::system;
use luks2::key::Passphrase;
use pki::InstallResult;
use sbolt::efi::enrollment;
use sbolt::keys::SigningPair;
use sbolt::keys::hierarchy::Bundle;
use tokio::sync::mpsc;
use wizard::artifact::Artifact;
use wizard::domain::profile::{CustomizationSpec, Profile};
use wizard::request::Request;

use crate::ipc::proto::provision::InstallProgress;
use crate::{disk, efi, medium, secrets, streaming};

/// Installs Muak to the disks assigned in `config`, with the given configuration.
struct PkiResult {
    client_result: InstallResult,
    auth_config: config::auth::State,
    server_pki: pki::Server,
}

pub async fn run(
    force: bool,
    allow_system_disk: bool,
    config: &system::Config,
    admin_csr_pem: &str,
    progress: mpsc::Sender<InstallProgress>,
) -> Result<InstallResult> {
    let targets = disk::resolve(&config.disks).await?;
    validate_disks(&targets, force, allow_system_disk, &progress).await?;
    let sb_hierarchy = generate_sb_hierarchy(config)?;
    let (luks_key, pki_result) = generate_keys(admin_csr_pem, &progress).await?;
    let booted_profile = medium::profile().context("failed to load booted profile")?;

    let tpm_available = tpm2::device::is_available(None);

    let partitions = volumes::partition(&targets, &progress).await?;
    volumes::format_luks(&partitions, &luks_key, &progress).await?;

    let (sections, sb_hierarchy) = build_and_deploy_efi(
        &partitions.efi,
        config,
        booted_profile,
        &luks_key,
        tpm_available,
        sb_hierarchy,
        &progress,
    )
    .await?;

    if tpm_available {
        let seal_result = secrets::seal_luks_key(&luks_key, &sections)?;
        match seal_result {
            secrets::SealResult::Sealed(token) => {
                secrets::write_token_to_devices(
                    &token,
                    &[&partitions.state, &partitions.ephemeral],
                )?;
                println!("LUKS key sealed to TPM2 with PCR#11 values");
            }
            secrets::SealResult::EspKey => {
                bail!("TPM available but seal returned EspKey");
            }
        }
    } else {
        println!("TPM2 unavailable, LUKS key written to ESP");
    }

    let mapped = volumes::open_luks(&partitions, &luks_key).await?;
    volumes::format_btrfs(&mapped).await?;

    initialize_state(
        &mapped.state,
        config,
        &pki_result,
        sb_hierarchy.as_ref(),
        &progress,
    )
    .await?;

    volumes::close_luks()?;

    enroll_secureboot_keys(sb_hierarchy.as_ref(), &progress).await?;

    Ok(pki_result.client_result)
}

async fn validate_disks(
    targets: &disk::Targets,
    force: bool,
    allow_system_disk: bool,
    progress: &mpsc::Sender<InstallProgress>,
) -> Result<()> {
    send_progress(progress, "Validating disks").await;
    let targets = targets.clone();

    tokio::task::spawn_blocking(move || {
        let checks = disk::preflight::checks(&targets, force, allow_system_disk);
        disk::preflight::enforce(&checks)?;
        disk::preflight::unmount_targets(&targets)
    })
    .await??;

    Ok(())
}

fn generate_sb_hierarchy(config: &system::Config) -> Result<Option<Bundle>> {
    if !config.host.secureboot {
        return Ok(None);
    }

    let setup_mode = firmware::secureboot::system().is_ok_and(|state| state.setup_mode);
    if !setup_mode {
        bail!(
            "Firmware is not in Setup Mode, cannot enroll Secure Boot keys. \
             Please reset your firmware to Setup Mode and try again or disable \
             the secureboot option in the config."
        );
    }

    Bundle::generate("Muak")
        .context("Failed to generate Secure Boot keys")
        .map(Some)
}

async fn enroll_secureboot_keys(
    sb_hierarchy: Option<&Bundle>,
    progress: &mpsc::Sender<InstallProgress>,
) -> Result<()> {
    if let Some(hierarchy) = sb_hierarchy {
        send_progress(progress, "Enrolling secureboot keys").await;
        enrollment::enroll(&firmware::system(), hierarchy)
            .context("Failed to enroll Secure Boot keys")?;
    }

    Ok(())
}

async fn generate_keys(
    admin_csr_pem: &str,
    progress: &mpsc::Sender<InstallProgress>,
) -> Result<(Passphrase, PkiResult)> {
    send_progress(progress, "Generating encryption keys").await;
    let luks_key = luks2::key::generate()?;

    send_progress(progress, "Generating PKI and signing CSR").await;
    let ca = pki::generate_ca()?;
    let server_pki = pki::generate_server_cert(&ca)?;
    let (client_result, auth_config) = pki::sign_admin_csr(admin_csr_pem, &ca)?;

    Ok((
        luks_key,
        PkiResult {
            client_result,
            auth_config,
            server_pki,
        },
    ))
}

async fn build_and_deploy_efi(
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

async fn initialize_state(
    dm_state: &str,
    config: &system::Config,
    pki_result: &PkiResult,
    sb_hierarchy: Option<&Bundle>,
    progress: &mpsc::Sender<InstallProgress>,
) -> Result<()> {
    send_progress(progress, "Initializing STATE partition").await;
    state::init(
        dm_state,
        config,
        &pki_result.auth_config,
        &pki_result.server_pki,
        sb_hierarchy,
    )?;

    Ok(())
}

async fn send_progress(progress: &mpsc::Sender<InstallProgress>, message: &str) {
    streaming::send_progress(
        progress,
        InstallProgress {
            message: message.to_owned(),
            ..Default::default()
        },
    )
    .await;
}
