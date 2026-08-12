//! Installation workflow orchestration.

pub mod pki;
mod state;

use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};

use anyhow::{Context as _, Result, bail};
use config::SystemConfig;
use pki::InstallResult;
use sbolt::efi::{enroll, setup_mode};
use sbolt::keys::SigningPair;
use sbolt::keys::hierarchy::Bundle;
use tokio::sync::mpsc;
use wizard::config::{Config, Sources, configure};
use wizard::profile::{CustomizationSpec, Profile};
use wizard::request::{Platform, Request};

use crate::constants::{DM_DATA, DM_STATE};
use crate::disk;
use crate::efi;
use crate::ipc::proto::provision::InstallProgress;
use crate::profile;
use crate::secrets;
use crate::streaming;

/// Installs Muak to the specified disks with the given configuration.
pub async fn run(
    system_disk: &str,
    data_disk: &str,
    force: bool,
    config: &SystemConfig,
    admin_csr_pem: &str,
    progress: mpsc::Sender<InstallProgress>,
) -> Result<InstallResult> {
    validate_disks(system_disk, data_disk, force, &progress).await?;
    let sb_hierarchy = generate_sb_hierarchy(config)?;
    let (luks_key, pki_result) = generate_keys(admin_csr_pem, &progress).await?;

    let tpm_available = tpm2::device::is_available(None);

    let partitions = partition_disks(system_disk, data_disk, &progress).await?;
    format_partitions(&partitions, &luks_key, &progress).await?;

    let signing = sb_hierarchy.as_ref().map(|hierarchy| SigningPair {
        signer: &hierarchy.db.signer,
        certificate: &hierarchy.db.certificate,
    });

    let sections = build_and_deploy_efi(
        &partitions.efi,
        &config.host.image,
        &config.host.extensions,
        &luks_key,
        tpm_available,
        signing.as_ref(),
        &progress,
    )
    .await?;

    if tpm_available {
        let seal_result = secrets::seal_luks_key(&luks_key, &sections)?;
        match seal_result {
            secrets::SealResult::Sealed(token) => {
                secrets::write_token_to_devices(&token, &[&partitions.state, &partitions.data])?;
                println!("LUKS key sealed to TPM2 with PCR#11 values");
            }
            secrets::SealResult::EspKey => {
                bail!("TPM available but seal returned EspKey");
            }
        }
    } else {
        println!("TPM2 unavailable, LUKS key written to ESP");
    }

    let (dm_state, dm_data) =
        open_luks_volumes(&partitions.state, &partitions.data, &luks_key).await?;
    format_btrfs_volumes(&dm_state, &dm_data, &progress).await?;

    initialize_state(
        &dm_state,
        config,
        &pki_result.auth_config,
        &pki_result.server_pki,
        sb_hierarchy.as_ref(),
        &progress,
    )
    .await?;

    close_luks_volumes()?;

    enroll_secureboot_keys(sb_hierarchy.as_ref(), &progress).await?;

    Ok(pki_result.client_result)
}

async fn validate_disks(
    system_disk: &str,
    data_disk: &str,
    force: bool,
    progress: &mpsc::Sender<InstallProgress>,
) -> Result<()> {
    send_progress(progress, "Validating disks").await;
    tokio::task::spawn_blocking({
        let system_disk = system_disk.to_owned();
        let data_disk = data_disk.to_owned();
        move || disk::install_target(&system_disk, &data_disk, force)
    })
    .await??;

    Ok(())
}

fn generate_sb_hierarchy(config: &SystemConfig) -> Result<Option<Bundle>> {
    if !config.host.secureboot {
        return Ok(None);
    }

    let setup_mode = setup_mode().unwrap_or(false);
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
        enroll(hierarchy).context("Failed to enroll Secure Boot keys")?;
    }

    Ok(())
}

struct PkiResult {
    client_result: InstallResult,
    auth_config: config::AuthConfig,
    server_pki: pki::Server,
}

async fn generate_keys(
    admin_csr_pem: &str,
    progress: &mpsc::Sender<InstallProgress>,
) -> Result<(Vec<u8>, PkiResult)> {
    send_progress(progress, "Generating encryption keys").await;
    let luks_key = pki::generate_luks_key()?;

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
    image: &str,
    extensions: &[String],
    luks_key: &[u8],
    tpm_available: bool,
    signing: Option<&SigningPair<'_>>,
    progress: &mpsc::Sender<InstallProgress>,
) -> Result<Vec<wizard::SectionInfo>> {
    send_progress(progress, "Building and deploying EFI").await;

    let install_profile = derive_install_profile(extensions)?;

    let (registry, installer, version) = image_parts(image)?;
    configure(Config {
        sources: Sources {
            registry,
            installer,
        },
        cache_dir: None,
    })
    .context("Failed to configure wizard")?;

    efi::mount(efi_part)?;

    let mut uki_file = efi::create(
        Path::new(efi::MOUNT_POINT),
        esp::arch::Arch::current().boot_path(),
    )?;

    let (mut overlay_r, mut overlay_w) = UnixStream::pair()?;

    let esp_root = PathBuf::from(efi::MOUNT_POINT);
    let demux = tokio::task::spawn_blocking(move || efi::extract_tar(&esp_root, &mut overlay_r));

    let request = Request::new(version, Platform::Metal)
        .uki(&mut uki_file)?
        .overlays(&mut overlay_w)?;

    let request = match signing {
        Some(pair) => request.sign(pair),
        None => request,
    };

    let metadata = request.build(&install_profile).await?;
    drop(overlay_w);

    if !tpm_available {
        efi::write_bytes(Path::new(efi::MOUNT_POINT), "luks", luks_key)?;
    }

    demux.await??;

    efi::unmount();

    Ok(metadata.sections)
}

fn derive_install_profile(extensions: &[String]) -> Result<Profile> {
    let booted = profile::load().context("failed to load booted profile")?;
    let customization =
        CustomizationSpec::new(extensions.to_vec()).context("invalid extensions")?;

    Ok(Profile::new(booted.overlay().cloned(), customization))
}

fn image_parts(image: &str) -> Result<(String, String, String)> {
    let colon = image
        .rfind(':')
        .context("invalid installer image: missing tag")?;
    let version = image.get(colon.saturating_add(1)..).unwrap_or_default();
    let path = image.get(..colon).unwrap_or_default();
    let slash = path
        .find('/')
        .context("invalid installer image: missing registry")?;
    let registry = path.get(..slash).unwrap_or_default();
    let installer = path.get(slash.saturating_add(1)..).unwrap_or_default();

    Ok((
        registry.to_owned(),
        installer.to_owned(),
        version.to_owned(),
    ))
}

struct PartitionInfo {
    efi: String,
    state: String,
    data: String,
}

/// Partitions both the system disk (EFI + STATE) and the data disk (DATA).
async fn partition_disks(
    system_disk: &str,
    data_disk: &str,
    progress: &mpsc::Sender<InstallProgress>,
) -> Result<PartitionInfo> {
    send_progress(progress, &format!("Partitioning {system_disk}")).await;

    let system_disk = system_disk.to_owned();
    let data_disk = data_disk.to_owned();

    tokio::task::spawn_blocking(move || partition_disks_blocking(&system_disk, &data_disk)).await?
}

fn partition_disks_blocking(system_disk: &str, data_disk: &str) -> Result<PartitionInfo> {
    disk::delete_all_partitions_blkpg(system_disk)?;
    disk::wipe(system_disk)?;
    let (efi_part, state_part) = disk::create_system_partitions(system_disk)?;

    if system_disk != data_disk {
        disk::delete_all_partitions_blkpg(data_disk)?;
        disk::wipe(data_disk)?;
    }
    let data_part = disk::create_data_partition(data_disk)?;

    Ok(PartitionInfo {
        efi: efi_part,
        state: state_part,
        data: data_part,
    })
}

async fn format_partitions(
    partitions: &PartitionInfo,
    luks_key: &[u8],
    progress: &mpsc::Sender<InstallProgress>,
) -> Result<()> {
    send_progress(progress, "Formatting partitions...").await;

    disk::format_efi_partition(&partitions.efi)?;

    let (state_result, data_result) = tokio::join!(
        flatten_join_result(tokio::task::spawn_blocking({
            let state_part = partitions.state.clone();
            let luks_key = luks_key.to_vec();
            move || {
                luks2::format(&state_part, &luks_key, "STATE")
                    .context("Failed to LUKS format STATE")
            }
        })),
        flatten_join_result(tokio::task::spawn_blocking({
            let data_part = partitions.data.clone();
            let luks_key = luks_key.to_vec();
            move || {
                luks2::format(&data_part, &luks_key, "DATA").context("Failed to LUKS format DATA")
            }
        })),
    );

    state_result?;
    data_result?;

    Ok(())
}

async fn open_luks_volumes(
    state_part: &str,
    data_part: &str,
    luks_key: &[u8],
) -> Result<(String, String)> {
    let (state_result, data_result) = tokio::join!(
        flatten_join_result(tokio::task::spawn_blocking({
            let state_part = state_part.to_owned();
            let luks_key = luks_key.to_vec();
            move || {
                luks2::open(&state_part, DM_STATE, &luks_key).context("Failed to open LUKS STATE")
            }
        })),
        flatten_join_result(tokio::task::spawn_blocking({
            let data_part = data_part.to_owned();
            let luks_key = luks_key.to_vec();
            move || luks2::open(&data_part, DM_DATA, &luks_key).context("Failed to open LUKS DATA")
        })),
    );

    state_result?;
    data_result?;

    Ok((
        format!("/dev/mapper/{DM_STATE}"),
        format!("/dev/mapper/{DM_DATA}"),
    ))
}

async fn format_btrfs_volumes(
    dm_state: &str,
    dm_data: &str,
    _progress: &mpsc::Sender<InstallProgress>,
) -> Result<()> {
    let (state_result, data_result) = tokio::join!(
        flatten_join_result(tokio::task::spawn_blocking({
            let dm_state = dm_state.to_owned();
            move || disk::format_btrfs_partition(&dm_state, "STATE")
        })),
        flatten_join_result(tokio::task::spawn_blocking({
            let dm_data = dm_data.to_owned();
            move || disk::format_btrfs_partition(&dm_data, "DATA")
        })),
    );

    state_result?;
    data_result?;

    Ok(())
}

async fn initialize_state(
    dm_state: &str,
    config: &SystemConfig,
    auth_config: &config::AuthConfig,
    server_pki: &pki::Server,
    sb_hierarchy: Option<&Bundle>,
    progress: &mpsc::Sender<InstallProgress>,
) -> Result<()> {
    send_progress(progress, "Initializing STATE partition").await;
    state::init(dm_state, config, auth_config, server_pki, sb_hierarchy)?;

    Ok(())
}

fn close_luks_volumes() -> Result<()> {
    luks2::close(DM_STATE).context("Failed to close LUKS STATE mapping")?;
    luks2::close(DM_DATA).context("Failed to close LUKS DATA mapping")?;

    Ok(())
}

async fn flatten_join_result<T>(handle: tokio::task::JoinHandle<Result<T>>) -> Result<T> {
    handle
        .await
        .map_err(|e| anyhow::anyhow!("Task panicked: {e}"))?
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
