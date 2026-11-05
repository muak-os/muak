//! Installation workflow orchestration.

mod deploy;
mod layout;
pub mod pki;
pub(crate) mod plan;
mod preflight;
mod secureboot;
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
use crate::{cache, check, disk, medium, secrets, streaming};

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
    validate_disks(&targets, config, force, allow_system_disk, &progress).await?;
    let enroll_secureboot = verify_secureboot(config)?;
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
        enroll_secureboot,
        &progress,
    )
    .await;

    disk::try_unmount(cache::EPHEMERAL_MOUNT);
    let closed = volumes::close_luks();

    let client_result = installed?;
    closed?;

    Ok(client_result)
}

async fn install_volumes(
    partitions: &volumes::PartitionInfo,
    luks_key: &Passphrase,
    pki_result: PkiResult,
    config: &system::Config,
    booted_profile: Profile,
    enroll_secureboot: bool,
    progress: &mpsc::Sender<InstallProgress>,
) -> Result<InstallResult> {
    let mapped = volumes::open_luks(partitions, luks_key).await?;
    volumes::format_btrfs(&mapped).await?;
    disk::mount_ephemeral(&mapped.ephemeral, cache::EPHEMERAL_MOUNT)?;

    cache::init();
    cache::enforce();

    let tpm_available = tpm2::device::is_available(None);
    let sb_hierarchy = if enroll_secureboot {
        Some(Bundle::generate("Muak").context("Failed to generate Secure Boot keys")?)
    } else {
        None
    };

    send_progress(progress, "Building and deploying EFI").await;

    let (sections, sb_hierarchy) = deploy::spawn(
        partitions.efi.clone(),
        config.host.clone(),
        booted_profile,
        luks_key.clone(),
        tpm_available,
        sb_hierarchy,
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
    config: &system::Config,
    force: bool,
    allow_system_disk: bool,
    progress: &mpsc::Sender<InstallProgress>,
) -> Result<()> {
    send_progress(progress, "Validating disks").await;
    let targets = targets.clone();
    let secureboot = config.host.secureboot;

    tokio::task::spawn_blocking(move || {
        let checks = preflight::checks(&targets, force, allow_system_disk, secureboot);
        check::enforce(&checks)?;
        disk::unmount_targets(&targets)
    })
    .await??;

    Ok(())
}

fn verify_secureboot(config: &system::Config) -> Result<bool> {
    let check = secureboot::check(config.host.secureboot);
    if !check.passed {
        bail!("{}", check.detail);
    }

    Ok(config.host.secureboot)
}

async fn enroll_secureboot_keys(
    sb_hierarchy: Option<&Bundle>,
    progress: &mpsc::Sender<InstallProgress>,
) -> Result<()> {
    let Some(hierarchy) = sb_hierarchy else {
        return Ok(());
    };

    send_progress(progress, "Enrolling secureboot keys").await;

    enrollment::enroll(&firmware::system(), hierarchy).context("Failed to enroll Secure Boot keys")
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
