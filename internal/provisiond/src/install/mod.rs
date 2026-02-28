//! Installation workflow orchestration.

mod pki;
mod state;

use std::path::Path;

use anyhow::{Context, Result, bail};
pub use pki::InstallResult;
use rustix::fs::sync;
use sysconfig::HostConfig;
use tokio::sync::mpsc;

use crate::constants;
use crate::constants::{DM_DATA, DM_STATE};
use crate::disk;
use crate::efi;
use crate::secrets;
use crate::services::proto::provision::InstallProgress;
use crate::streaming;
use crate::uki::Uki;

/// Installs Muak to the specified disk with the given configuration.
pub async fn run(
    disk_path: &str,
    force: bool,
    config: &HostConfig,
    admin_csr_pem: &str,
    progress: mpsc::Sender<InstallProgress>,
) -> Result<InstallResult> {
    streaming::send_progress(
        &progress,
        InstallProgress {
            message: format!("Validating disk {}", disk_path),
            ..Default::default()
        },
    )
    .await;
    let disk_path = disk_path.to_owned();
    tokio::task::spawn_blocking({
        let disk_path = disk_path.clone();
        move || disk::validate_install_target(&disk_path, force)
    })
    .await??;

    streaming::send_progress(
        &progress,
        InstallProgress {
            message: "Enrolling secureboot keys".to_string(),
            ..Default::default()
        },
    )
    .await;
    let sb_hierarchy = generate_sb_hierarchy(config)?;
    if let Some(ref hierarchy) = sb_hierarchy {
        sbolt::efi::enroll_keys(hierarchy).context("Failed to enroll Secure Boot keys")?;
    }

    streaming::send_progress(
        &progress,
        InstallProgress {
            message: "Generating encryption keys".to_string(),
            ..Default::default()
        },
    )
    .await;
    let luks_key = pki::generate_luks_key()?;

    streaming::send_progress(
        &progress,
        InstallProgress {
            message: "Generating PKI and signing CSR".to_string(),
            ..Default::default()
        },
    )
    .await;
    let ca = pki::generate_ca()?;
    let server_pki = pki::generate_server_cert(&ca)?;
    let (client_result, auth_config) = pki::sign_admin_csr(admin_csr_pem, &ca)?;

    streaming::send_progress(
        &progress,
        InstallProgress {
            message: format!("Pulling installer image: {}", config.system.image),
            ..Default::default()
        },
    )
    .await;
    let work_dir = Path::new(constants::INSTALL_DIR);
    let mut uki = Uki::prepare(&config.system.image, &config.system.extensions, work_dir).await?;
    let staged_uki = work_dir.join("staged.efi");

    let seal_result = secrets::seal_luks_key(&luks_key, &mut uki)?;

    streaming::send_progress(
        &progress,
        InstallProgress {
            message: "Building UKI".to_string(),
            ..Default::default()
        },
    )
    .await;
    uki.build(&staged_uki, sb_hierarchy.as_ref())?;

    streaming::send_progress(
        &progress,
        InstallProgress {
            message: format!("Partitioning disk {}", disk_path),
            ..Default::default()
        },
    )
    .await;
    let (efi_part, state_part, data_part) = tokio::task::spawn_blocking({
        let disk_path = disk_path.clone();
        move || -> Result<_> {
            disk::delete_all_partitions_blkpg(&disk_path)?;
            disk::wipe_disk(&disk_path)?;
            disk::create_partitions(&disk_path)
        }
    })
    .await??;

    streaming::send_progress(
        &progress,
        InstallProgress {
            message: "Formatting partitions...".to_string(),
            ..Default::default()
        },
    )
    .await;
    disk::format_efi_partition(&efi_part)?;

    let (luks_key_a, luks_key_b) = (luks_key.clone(), luks_key.clone());
    let (state_part_a, data_part_a) = (state_part.clone(), data_part.clone());
    tokio::try_join!(
        tokio::task::spawn_blocking(move || luks2::format(&state_part_a, &luks_key_a, "STATE")
            .context("Failed to LUKS format STATE")),
        tokio::task::spawn_blocking(move || luks2::format(&data_part_a, &luks_key_b, "DATA")
            .context("Failed to LUKS format DATA")),
    )
    .map_err(|e| anyhow::anyhow!("Format task panicked: {}", e))
    .and_then(|(a, b)| a.and(b))?;

    if let secrets::SealResult::Sealed(ref token) = seal_result {
        secrets::write_token_to_devices(token, &[&state_part, &data_part])?;
        println!("LUKS key sealed to TPM2 with PCR#11 values");
    }

    let (luks_key_c, luks_key_d) = (luks_key.clone(), luks_key.clone());
    let (state_part_b, data_part_b) = (state_part.clone(), data_part.clone());
    tokio::try_join!(
        tokio::task::spawn_blocking(move || luks2::open(&state_part_b, DM_STATE, &luks_key_c)
            .context("Failed to open LUKS STATE")),
        tokio::task::spawn_blocking(move || luks2::open(&data_part_b, DM_DATA, &luks_key_d)
            .context("Failed to open LUKS DATA")),
    )
    .map_err(|e| anyhow::anyhow!("Open task panicked: {}", e))
    .and_then(|(a, b)| a.and(b))?;

    let dm_state = format!("/dev/mapper/{}", DM_STATE);
    let dm_data = format!("/dev/mapper/{}", DM_DATA);
    let (dm_state_a, dm_data_a) = (dm_state.clone(), dm_data.clone());
    tokio::try_join!(
        tokio::task::spawn_blocking(move || disk::format_btrfs_partition(&dm_state_a, "STATE")),
        tokio::task::spawn_blocking(move || disk::format_btrfs_partition(&dm_data_a, "DATA")),
    )
    .map_err(|e| anyhow::anyhow!("Btrfs format task panicked: {}", e))
    .and_then(|(a, b)| a.and(b))?;

    streaming::send_progress(
        &progress,
        InstallProgress {
            message: "Deploying UKI to EFI partition".to_string(),
            ..Default::default()
        },
    )
    .await;
    efi::deploy(&efi_part, &staged_uki)?;

    streaming::send_progress(
        &progress,
        InstallProgress {
            message: "Initializing STATE partition".to_string(),
            ..Default::default()
        },
    )
    .await;
    state::init(
        &dm_state,
        config,
        &auth_config,
        &server_pki,
        sb_hierarchy.as_ref(),
    )?;

    luks2::close(DM_STATE).context("Failed to close LUKS STATE mapping")?;
    luks2::close(DM_DATA).context("Failed to close LUKS DATA mapping")?;

    if let Err(e) = std::fs::remove_dir_all(work_dir) {
        eprintln!("Failed to cleanup work dir: {}", e);
    }

    sync();

    Ok(client_result)
}

/// Generates the Secure Boot key hierarchy if secureboot is enabled in config.
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
