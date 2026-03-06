//! Installation workflow orchestration.

mod pki;
mod state;

use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
pub use pki::InstallResult;
use rustix::fs::sync;
use sysconfig::HostConfig;
use tokio::sync::mpsc;

use crate::constants::{DM_DATA, DM_STATE};
use crate::disk;
use crate::efi;
use crate::ipc::proto::provision::InstallProgress;
use crate::secrets;
use crate::streaming;
use crate::uki::Uki;

/// Working directory for installation operations.
const INSTALL_DIR: &str = "/run/install";

/// Installs Muak to the specified disk with the given configuration.
pub async fn run(
    disk_path: &str,
    force: bool,
    config: &HostConfig,
    admin_csr_pem: &str,
    progress: mpsc::Sender<InstallProgress>,
) -> Result<InstallResult> {
    let sb_hierarchy = setup_security(disk_path, force, config, &progress).await?;
    let (luks_key, pki_result) = generate_keys(admin_csr_pem, &progress).await?;
    let uki = prepare_uki(
        &config.system.image,
        &config.system.extensions,
        &luks_key,
        &progress,
        sb_hierarchy.as_ref(),
    )
    .await?;

    let partitions = partition_disk(disk_path, &progress).await?;
    format_partitions(&partitions, &luks_key, &progress).await?;

    match uki.seal_result {
        secrets::SealResult::Sealed(token) => {
            secrets::write_token_to_devices(
                &token,
                &[&partitions.state_part, &partitions.data_part],
            )?;
            println!("LUKS key sealed to TPM2 with PCR#11 values");
        }
        secrets::SealResult::Embedded => {}
    }

    let (dm_state, dm_data) =
        open_luks_volumes(&partitions.state_part, &partitions.data_part, &luks_key).await?;
    format_btrfs_volumes(&dm_state, &dm_data, &progress).await?;

    deploy_uki(&partitions.efi_part, &uki.staged_path, &progress).await?;
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
    cleanup_work_dir(uki.work_dir)?;

    sync();

    Ok(pki_result.client_result)
}

async fn setup_security(
    disk_path: &str,
    force: bool,
    config: &HostConfig,
    progress: &mpsc::Sender<InstallProgress>,
) -> Result<Option<sbolt::keys::KeyHierarchy>> {
    send_progress(progress, &format!("Validating disk {}", disk_path)).await;
    tokio::task::spawn_blocking({
        let disk_path = disk_path.to_string();
        move || disk::validate_install_target(&disk_path, force)
    })
    .await??;

    let sb_hierarchy = generate_sb_hierarchy(config)?;
    if let Some(ref hierarchy) = sb_hierarchy {
        send_progress(progress, "Enrolling secureboot keys").await;
        sbolt::efi::enroll_keys(hierarchy).context("Failed to enroll Secure Boot keys")?;
    }

    Ok(sb_hierarchy)
}

fn generate_sb_hierarchy(config: &HostConfig) -> Result<Option<sbolt::keys::KeyHierarchy>> {
    if !config.system.secureboot {
        return Ok(None);
    }

    let setup_mode = sbolt::efi::get_setup_mode().unwrap_or(false);
    if !setup_mode {
        bail!(
            "Firmware is not in Setup Mode, cannot enroll Secure Boot keys. \
             Please reset your firmware to Setup Mode and try again or disable the secureboot option in the config."
        );
    }

    let hierarchy = sbolt::keys::KeyHierarchy::generate("Muak")
        .context("Failed to generate Secure Boot keys")?;

    Ok(Some(hierarchy))
}

struct PkiResult {
    client_result: InstallResult,
    auth_config: sysconfig::AuthConfig,
    server_pki: pki::ServerPki,
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

struct PreparedUki {
    work_dir: PathBuf,
    staged_path: PathBuf,
    seal_result: secrets::SealResult,
}

async fn prepare_uki(
    image: &str,
    extensions: &[String],
    luks_key: &[u8],
    progress: &mpsc::Sender<InstallProgress>,
    sb_hierarchy: Option<&sbolt::keys::KeyHierarchy>,
) -> Result<PreparedUki> {
    send_progress(progress, &format!("Pulling installer image: {}", image)).await;
    let work_dir = Path::new(INSTALL_DIR);
    let mut uki = Uki::prepare(image, extensions, work_dir).await?;
    let staged_path = work_dir.join("staged.efi");

    let seal_result = secrets::seal_luks_key(luks_key, &mut uki)?;

    send_progress(progress, "Building UKI").await;
    uki.build(&staged_path, sb_hierarchy)?;

    Ok(PreparedUki {
        work_dir: work_dir.to_path_buf(),
        staged_path,
        seal_result,
    })
}

struct PartitionInfo {
    efi_part: String,
    state_part: String,
    data_part: String,
}

async fn partition_disk(
    disk_path: &str,
    progress: &mpsc::Sender<InstallProgress>,
) -> Result<PartitionInfo> {
    send_progress(progress, &format!("Partitioning disk {}", disk_path)).await;
    let result = tokio::task::spawn_blocking({
        let disk_path = disk_path.to_string();
        move || -> Result<_> {
            disk::delete_all_partitions_blkpg(&disk_path)?;
            disk::wipe_disk(&disk_path)?;
            disk::create_partitions(&disk_path)
        }
    })
    .await??;

    Ok(PartitionInfo {
        efi_part: result.0,
        state_part: result.1,
        data_part: result.2,
    })
}

async fn format_partitions(
    partitions: &PartitionInfo,
    luks_key: &[u8],
    progress: &mpsc::Sender<InstallProgress>,
) -> Result<()> {
    send_progress(progress, "Formatting partitions...").await;

    disk::format_efi_partition(&partitions.efi_part)?;

    let (state_result, data_result) = tokio::join!(
        flatten_join_result(tokio::task::spawn_blocking({
            let state_part = partitions.state_part.clone();
            let luks_key = luks_key.to_vec();
            move || {
                luks2::format(&state_part, &luks_key, "STATE")
                    .context("Failed to LUKS format STATE")
            }
        })),
        flatten_join_result(tokio::task::spawn_blocking({
            let data_part = partitions.data_part.clone();
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
            let state_part = state_part.to_string();
            let luks_key = luks_key.to_vec();
            move || {
                luks2::open(&state_part, DM_STATE, &luks_key).context("Failed to open LUKS STATE")
            }
        })),
        flatten_join_result(tokio::task::spawn_blocking({
            let data_part = data_part.to_string();
            let luks_key = luks_key.to_vec();
            move || luks2::open(&data_part, DM_DATA, &luks_key).context("Failed to open LUKS DATA")
        })),
    );

    state_result?;
    data_result?;

    Ok((
        format!("/dev/mapper/{}", DM_STATE),
        format!("/dev/mapper/{}", DM_DATA),
    ))
}

async fn format_btrfs_volumes(
    dm_state: &str,
    dm_data: &str,
    _progress: &mpsc::Sender<InstallProgress>,
) -> Result<()> {
    let (state_result, data_result) = tokio::join!(
        flatten_join_result(tokio::task::spawn_blocking({
            let dm_state = dm_state.to_string();
            move || disk::format_btrfs_partition(&dm_state, "STATE")
        })),
        flatten_join_result(tokio::task::spawn_blocking({
            let dm_data = dm_data.to_string();
            move || disk::format_btrfs_partition(&dm_data, "DATA")
        })),
    );

    state_result?;
    data_result?;

    Ok(())
}

async fn deploy_uki(
    efi_part: &str,
    staged_path: &Path,
    progress: &mpsc::Sender<InstallProgress>,
) -> Result<()> {
    send_progress(progress, "Deploying UKI to EFI partition").await;
    efi::deploy(efi_part, staged_path)?;
    Ok(())
}

async fn initialize_state(
    dm_state: &str,
    config: &HostConfig,
    auth_config: &sysconfig::AuthConfig,
    server_pki: &pki::ServerPki,
    sb_hierarchy: Option<&sbolt::keys::KeyHierarchy>,
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

fn cleanup_work_dir(work_dir: PathBuf) -> Result<()> {
    if let Err(e) = std::fs::remove_dir_all(&work_dir) {
        eprintln!("Failed to cleanup work dir: {}", e);
    }
    Ok(())
}

async fn flatten_join_result<T>(handle: tokio::task::JoinHandle<Result<T>>) -> Result<T> {
    handle
        .await
        .map_err(|e| anyhow::anyhow!("Task panicked: {}", e))?
}

async fn send_progress(progress: &mpsc::Sender<InstallProgress>, message: &str) {
    streaming::send_progress(
        progress,
        InstallProgress {
            message: message.to_string(),
            ..Default::default()
        },
    )
    .await;
}
