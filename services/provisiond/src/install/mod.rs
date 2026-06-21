//! Installation workflow orchestration.

mod pki;
mod state;

use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use config::SystemConfig;
use wizard::artifact::Artifact;
use wizard::build;
use wizard::profile::Profile;
use wizard::request::{Platform, Request};
use wizard::resolve::Config;
pub use pki::InstallResult;
use rustix::fs::sync;
use sbolt::keys::SigningPair;
use tokio::sync::mpsc;

use crate::constants::{DM_DATA, DM_STATE};
use crate::disk;
use crate::efi;
use crate::ipc::proto::provision::InstallProgress;
use crate::profile;
use crate::secrets;
use crate::streaming;

/// Working directory for installation operations.
const INSTALL_DIR: &str = "/run/install";

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
    let uki = prepare_uki(
        &config.host.image,
        &config.host.extensions,
        &luks_key,
        &progress,
        sb_hierarchy.as_ref(),
    )
    .await?;

    let partitions = partition_disks(system_disk, data_disk, &progress).await?;
    format_partitions(&partitions, &luks_key, &progress).await?;

    match uki.seal_result {
        secrets::SealResult::Sealed(ref token) => {
            secrets::write_token_to_devices(
                token,
                &[&partitions.state_part, &partitions.data_part],
            )?;
            println!("LUKS key sealed to TPM2 with PCR#11 values");
        }
        secrets::SealResult::EspKey => {
            println!("TPM2 unavailable, LUKS key will be written to ESP");
        }
    }

    let (dm_state, dm_data) =
        open_luks_volumes(&partitions.state_part, &partitions.data_part, &luks_key).await?;
    format_btrfs_volumes(&dm_state, &dm_data, &progress).await?;

    deploy_uki(
        &partitions.efi_part,
        &uki.staged_path,
        &uki.esp_files,
        &uki.luks_key,
        &progress,
    )
    .await?;
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

    enroll_secureboot_keys(sb_hierarchy.as_ref(), &progress).await?;

    sync();

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
        let system_disk = system_disk.to_string();
        let data_disk = data_disk.to_string();
        move || disk::validate_install_target(&system_disk, &data_disk, force)
    })
    .await??;

    Ok(())
}

fn generate_sb_hierarchy(config: &SystemConfig) -> Result<Option<sbolt::keys::hierarchy::Bundle>> {
    if !config.host.secureboot {
        return Ok(None);
    }

    let setup_mode = sbolt::efi::setup_mode().unwrap_or(false);
    if !setup_mode {
        bail!(
            "Firmware is not in Setup Mode, cannot enroll Secure Boot keys. \
             Please reset your firmware to Setup Mode and try again or disable \
             the secureboot option in the config."
        );
    }

    sbolt::keys::hierarchy::Bundle::generate("Muak")
        .context("Failed to generate Secure Boot keys")
        .map(Some)
}

async fn enroll_secureboot_keys(
    sb_hierarchy: Option<&sbolt::keys::hierarchy::Bundle>,
    progress: &mpsc::Sender<InstallProgress>,
) -> Result<()> {
    if let Some(hierarchy) = sb_hierarchy {
        send_progress(progress, "Enrolling secureboot keys").await;
        sbolt::efi::enroll(hierarchy).context("Failed to enroll Secure Boot keys")?;
    }

    Ok(())
}

struct PkiResult {
    client_result: InstallResult,
    auth_config: config::AuthConfig,
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
    esp_files: Vec<esp::EspFile>,
    seal_result: secrets::SealResult,
    luks_key: Option<Vec<u8>>,
}

async fn prepare_uki(
    image: &str,
    extensions: &[String],
    luks_key: &[u8],
    progress: &mpsc::Sender<InstallProgress>,
    sb_hierarchy: Option<&sbolt::keys::hierarchy::Bundle>,
) -> Result<PreparedUki> {
    let install_profile = derive_install_profile(extensions)?;

    send_progress(progress, &format!("Pulling installer image: {}", image)).await;
    let output_dir = Path::new(INSTALL_DIR).join("assets");
    let (registry, installer, version) = image_parts(image)?;
    let config = Config {
        sources: wizard::resolve::Sources {
            registry,
            installer,
        },
    };
    let request = Request {
        version,
        platform: Platform::Metal,
        arch: None,
        artifacts: vec![Artifact::Uki],
    };

    let signing = sb_hierarchy.as_ref().map(|h| SigningPair {
        signer: &h.db.signer,
        certificate: &h.db.certificate,
    });

    let staged_path = output_dir.join("signed.efi");
    let uki_file = std::fs::File::create(&staged_path)
        .with_context(|| format!("create UKI file {}", staged_path.display()))?;
    let mut uki_file = std::io::BufWriter::new(uki_file);

    let writers = build::ArtifactWriters {
        uki: Some(&mut uki_file),
        kernel: None,
        cmdline: None,
        initramfs: None,
        iso: None,
        raw: None,
    };

    let meta = build::artifacts(&request, &install_profile, &config, signing.as_ref(), writers)
        .await
        .context("wizard build artifacts")?;

    drop(uki_file);

    let uki_bytes = std::fs::read(&staged_path)
        .with_context(|| format!("read UKI back from {}", staged_path.display()))?;
    let seal_result = secrets::seal_luks_key(luks_key, &uki_bytes, &meta.sections)?;

    let luks_file = if matches!(seal_result, secrets::SealResult::EspKey) {
        Some(luks_key.to_vec())
    } else {
        None
    };

    Ok(PreparedUki {
        work_dir: Path::new(INSTALL_DIR).to_path_buf(),
        staged_path,
        esp_files: meta.overlay_files,
        seal_result,
        luks_key: luks_file,
    })
}

fn derive_install_profile(extensions: &[String]) -> Result<Profile> {
    let booted = profile::load().context("failed to load booted profile")?;
    let customization = wizard::profile::CustomizationSpec::new(extensions.to_vec())
        .context("invalid extensions")?;

    Ok(Profile::new(booted.overlay().cloned(), customization))
}

fn image_parts(image: &str) -> Result<(String, String, String)> {
    let colon = image
        .rfind(':')
        .context("invalid installer image: missing tag")?;
    let version = &image[colon + 1..];
    let path = &image[..colon];
    let slash = path
        .find('/')
        .context("invalid installer image: missing registry")?;
    let registry = &path[..slash];
    let installer = &path[slash + 1..];

    Ok((
        registry.to_owned(),
        installer.to_owned(),
        version.to_owned(),
    ))
}

struct PartitionInfo {
    efi_part: String,
    state_part: String,
    data_part: String,
}

/// Partitions both the system disk (EFI + STATE) and the data disk (DATA).
async fn partition_disks(
    system_disk: &str,
    data_disk: &str,
    progress: &mpsc::Sender<InstallProgress>,
) -> Result<PartitionInfo> {
    send_progress(progress, &format!("Partitioning {}", system_disk)).await;

    tokio::task::spawn_blocking({
        let system_disk = system_disk.to_string();
        let data_disk = data_disk.to_string();
        move || {
            disk::delete_all_partitions_blkpg(&system_disk)?;
            disk::wipe_disk(&system_disk)?;
            let (efi_part, state_part) = disk::create_system_partitions(&system_disk)?;

            if system_disk != data_disk {
                disk::delete_all_partitions_blkpg(&data_disk)?;
                disk::wipe_disk(&data_disk)?;
            }
            let data_part = disk::create_data_partition(&data_disk)?;

            Ok(PartitionInfo {
                efi_part,
                state_part,
                data_part,
            })
        }
    })
    .await?
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
    esp_files: &[esp::EspFile],
    luks_key: &Option<Vec<u8>>,
    progress: &mpsc::Sender<InstallProgress>,
) -> Result<()> {
    send_progress(progress, "Deploying UKI to EFI partition").await;

    let mut all_files = esp_files.to_vec();
    if let Some(key) = luks_key {
        all_files.push(esp::EspFile {
            path: "luks".into(),
            data: key.clone(),
        });
    }

    efi::deploy(efi_part, staged_path, &all_files)?;

    Ok(())
}

async fn initialize_state(
    dm_state: &str,
    config: &SystemConfig,
    auth_config: &config::AuthConfig,
    server_pki: &pki::ServerPki,
    sb_hierarchy: Option<&sbolt::keys::hierarchy::Bundle>,
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
