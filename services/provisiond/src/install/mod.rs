//! Installation workflow orchestration.

mod deploy;
mod layout;
pub mod pki;
pub(crate) mod plan;
mod state;
mod volumes;

use anyhow::{Context as _, Result, bail};
use config::system;
use luks2::key::Passphrase;
use pki::InstallResult;
use sbolt::efi::enrollment;
use sbolt::keys::hierarchy::Bundle;
use tokio::sync::mpsc;
use uki::measure::MeasuredSection;
use wizard::domain::profile::Profile;

use crate::ipc::proto::provision::InstallProgress;
use crate::{cache, disk, medium, secrets, streaming};

struct PkiResult {
    client_result: InstallResult,
    auth_config: config::auth::State,
    server_pki: pki::Server,
}

/// Installs Muak to the disks assigned in `config`, with the given configuration.
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

    let partitions = volumes::partition(&targets, &progress).await?;
    volumes::format_luks(&partitions, &luks_key, &progress).await?;

    let installed = install_volumes(
        &partitions,
        &luks_key,
        pki_result,
        config,
        booted_profile,
        sb_hierarchy,
        &progress,
    )
    .await;

    volumes::unmount_ephemeral();
    let closed = volumes::close_luks();

    let client_result = installed?;
    closed?;

    Ok(client_result)
}

async fn install_volumes(
    partitions: &volumes::PartitionInfo,
    luks_key: &[u8],
    pki_result: PkiResult,
    config: &system::Config,
    booted_profile: Profile,
    sb_hierarchy: Option<Bundle>,
    progress: &mpsc::Sender<InstallProgress>,
) -> Result<InstallResult> {
    let mapped = volumes::open_luks(partitions, luks_key).await?;
    volumes::format_btrfs(&mapped).await?;
    volumes::mount_ephemeral(&mapped.ephemeral)?;

    cache::init();
    cache::enforce();

    let tpm_available = tpm2::device::is_available(None);

    let (sections, sb_hierarchy) = deploy::build_and_deploy_efi(
        &partitions.efi,
        config,
        booted_profile,
        luks_key,
        tpm_available,
        sb_hierarchy,
        progress,
    )
    .await?;

    seal_luks_key(luks_key, tpm_available, partitions, &sections)?;

    initialize_state(
        &mapped.state,
        config,
        &pki_result,
        sb_hierarchy.as_ref(),
        progress,
    )
    .await?;

    cache::enforce();

    enroll_secureboot_keys(sb_hierarchy.as_ref(), progress).await?;

    Ok(pki_result.client_result)
}

fn seal_luks_key(
    luks_key: &[u8],
    tpm_available: bool,
    partitions: &volumes::PartitionInfo,
    sections: &[MeasuredSection],
) -> Result<()> {
    if !tpm_available {
        println!("TPM2 unavailable, LUKS key written to ESP");
        return Ok(());
    }

    match secrets::seal_luks_key(luks_key, sections)? {
        secrets::SealResult::Sealed(token) => {
            secrets::write_token_to_devices(&token, &[&partitions.state, &partitions.ephemeral])?;
            println!("LUKS key sealed to TPM2 with PCR#11 values");
            Ok(())
        }
        secrets::SealResult::EspKey => bail!("TPM available but seal returned EspKey"),
    }
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
