//! Installation workflow orchestration.

mod deploy;
mod keys;
mod layout;
pub(crate) mod pki;
pub(crate) mod plan;
mod preflight;
mod secureboot;
mod state;
mod volumes;

use anyhow::{Context as _, Result, bail};
use blockdev::types::Disk;
use config::system;
use keys::PkiResult;
use luks2::key::Passphrase;
use pki::InstallResult;
use sbolt::efi::enrollment;
use sbolt::keys::hierarchy::Bundle;
use tokio::sync::mpsc;
use wizard::domain::profile::{CustomizationSpec, Profile};

use crate::ipc::proto::provision::InstallProgress;
use crate::{cache, check, disk, medium, streaming};

/// Installs Muak to the disks assigned in `config`, with the given configuration.
pub(crate) async fn run(
    force: bool,
    allow_system_disk: bool,
    config: &system::Config,
    admin_csr_pem: &str,
    progress: mpsc::Sender<InstallProgress>,
) -> Result<InstallResult> {
    let inventory = disk::list_disks().await?;
    let targets = disk::target::resolve_with(&config.disk, &inventory)?;
    validate_disks(
        &targets,
        inventory,
        config,
        force,
        allow_system_disk,
        &progress,
    )
    .await?;
    let enroll_secureboot = verify_secureboot(config)?;
    let booted_profile = medium::profile().context("failed to load booted profile")?;
    let install_profile = derive_install_profile(&booted_profile, &config.host.extensions)?;
    preflight::verify_registry(&install_profile, config, &progress).await?;
    let (luks_key, pki_result) = keys::generate_keys(admin_csr_pem, &progress).await?;

    let partitions = volumes::partition(&targets, &progress).await?;
    volumes::format_luks(&partitions, &luks_key, &progress).await?;

    let installed = install_volumes(
        &partitions,
        &luks_key,
        pki_result,
        config,
        install_profile,
        enroll_secureboot,
        &progress,
    )
    .await;

    disk::mount::try_unmount(cache::EPHEMERAL_MOUNT);
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
    install_profile: Profile,
    enroll_secureboot: bool,
    progress: &mpsc::Sender<InstallProgress>,
) -> Result<InstallResult> {
    let mapped = volumes::open_luks(partitions, luks_key).await?;
    volumes::format_btrfs(&mapped).await?;
    disk::mount::ephemeral(&mapped.ephemeral, cache::EPHEMERAL_MOUNT)?;

    cache::init();
    cache::enforce();

    let tpm_available = tpm2::linux::is_available(None);
    let sb_hierarchy = if enroll_secureboot {
        Some(Bundle::generate("Muak").context("Failed to generate Secure Boot keys")?)
    } else {
        None
    };

    send_progress(progress, "Building and deploying EFI").await;

    let (sections, sb_hierarchy) = deploy::spawn(
        partitions.efi.clone(),
        config.host.clone(),
        install_profile,
        luks_key.clone(),
        tpm_available,
        sb_hierarchy,
    )
    .await?;

    keys::seal_luks_key(luks_key, tpm_available, partitions, &sections)?;

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

async fn validate_disks(
    targets: &disk::target::Targets,
    inventory: Vec<Disk>,
    config: &system::Config,
    force: bool,
    allow_system_disk: bool,
    progress: &mpsc::Sender<InstallProgress>,
) -> Result<()> {
    send_progress(progress, "Validating disks").await;
    let targets = targets.clone();
    let secureboot = config.host.secureboot;

    tokio::task::spawn_blocking(move || {
        let checks = preflight::checks(&targets, &inventory, force, allow_system_disk, secureboot)?;
        check::enforce(&checks)?;
        disk::mount::unmount_targets(&targets)
    })
    .await??;

    Ok(())
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

fn verify_secureboot(config: &system::Config) -> Result<bool> {
    if let Some(failed) = secureboot::system_checks(config.host.secureboot)
        .iter()
        .find(|check| !check.passed)
    {
        bail!("{}", failed.detail);
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

pub(super) async fn send_progress(progress: &mpsc::Sender<InstallProgress>, message: &str) {
    streaming::send_progress(
        progress,
        InstallProgress {
            message: message.to_owned(),
            ..Default::default()
        },
    )
    .await;
}
