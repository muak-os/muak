//! Installation workflow implementation for deploying Muak to a disk.

use std::path::Path;

use anyhow::{Context, Result, bail};
use der::EncodePem;
use der::pem::LineEnding;
use rustix::fs::sync;
use rustix::mount::{MountFlags, mount};
use sysconfig::{AuthConfig, AuthUser, HostConfig, Permission};

use sbolt::keys::{KeyHierarchy, save_key_hierarchy};

use crate::constants;
use crate::disk;
use crate::uki::{self, Uki};

/// System installation status.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InstallationStatus {
    Live,
    Installed,
}

/// Result of a successful installation containing PKI materials.
pub struct InstallResult {
    pub ca_pem: String,
    pub admin_cert_pem: String,
}

/// Internal PKI materials to store on the server.
struct ServerPki {
    pub ca_pem: String,
    pub ca_key_pem: String,
    pub server_cert_pem: String,
    pub server_key_pem: String,
}

/// Returns the current installation status of the system.
pub fn status() -> InstallationStatus {
    if Path::new(sysconfig::CONFIG_PATH).exists() {
        InstallationStatus::Installed
    } else {
        InstallationStatus::Live
    }
}

/// Installs Muak to the specified disk with the given configuration.
pub fn install(
    disk_path: &str,
    force: bool,
    config: &HostConfig,
    admin_csr_pem: &str,
) -> Result<InstallResult> {
    kmsg::info!("Starting installation to {}", disk_path);

    validate_disk(disk_path, force)?;

    let sb_hierarchy = if config.system.secureboot {
        let setup_mode = sbolt::efi::get_setup_mode().unwrap_or(false);
        if !setup_mode {
            bail!(
                "Firmware is not in Setup Mode, cannot enroll Secure Boot keys. Please reset your firmware to Setup Mode and try again or disable the secureboot option in the config."
            );
        }

        let hierarchy =
            KeyHierarchy::generate("Muak").context("Failed to generate Secure Boot keys")?;

        Some(hierarchy)
    } else {
        kmsg::info!("Secure Boot disabled, skipping key generation and enrollment");
        None
    };

    let (client_result, server_pki, config_with_auth) =
        generate_pki_and_sign_csr(admin_csr_pem, config)?;

    let work_dir = Path::new(constants::INSTALL_DIR);
    let components = Uki::prepare(
        &config_with_auth.system.image,
        &config_with_auth.system.extensions,
        work_dir,
    )?;
    let staged_uki = work_dir.join("staged.efi");

    components.build(&staged_uki)?;

    if let Some(ref hierarchy) = sb_hierarchy {
        Uki::sign(&staged_uki, hierarchy)?;
    }

    // Enroll Secure Boot keys only after the UKI is fully built and signed.
    // This prevents leaving the firmware in a non-Setup Mode state if earlier
    // steps (e.g. image pull) fail, which would block retries.
    if let Some(ref hierarchy) = sb_hierarchy {
        sbolt::efi::enroll_keys(hierarchy).context("Failed to enroll Secure Boot keys")?;
        kmsg::info!("Secure Boot keys enrolled");
    }

    disk::delete_all_partitions_blkpg(disk_path)?;
    disk::wipe_disk(disk_path)?;
    let (efi_part, state_part, data_part) = disk::create_partitions(disk_path)?;

    disk::format_efi_partition(&efi_part)?;
    disk::format_btrfs_partition(&state_part, "STATE")?;
    disk::format_btrfs_partition(&data_part, "DATA")?;

    deploy_uki_to_efi(&efi_part, &staged_uki)?;
    init_state_partition(
        &state_part,
        &config_with_auth,
        &server_pki,
        sb_hierarchy.as_ref(),
    )?;

    if let Err(e) = uki::cleanup_dir(work_dir) {
        kmsg::warn!("Failed to cleanup work dir: {}", e);
    }

    sync();
    kmsg::info!("Installation completed successfully!");

    Ok(client_result)
}

/// Validates the disk is suitable for installation.
fn validate_disk(disk_path: &str, force: bool) -> Result<()> {
    if !force && status() != InstallationStatus::Live {
        bail!(
            "Cannot install from an already-installed system. Boot from live ISO or use --force."
        );
    }

    if !Path::new(disk_path).exists() {
        bail!("Disk '{}' does not exist", disk_path);
    }

    disk::validate_block_device(disk_path)?;
    disk::validate_disk_size(disk_path)?;

    let mounted = disk::get_disk_mounts(disk_path);
    if !mounted.is_empty() && !force {
        bail!(
            "Cannot install: {} is mounted at {}. Use --force to unmount automatically.",
            mounted[0].device,
            mounted[0].mount_point
        );
    }

    sync();

    disk::unmount_all(&mounted)?;

    if !force && disk::has_existing_partitions(disk_path)? {
        bail!(
            "Disk '{}' has existing partitions. Use --force to overwrite.",
            disk_path
        );
    }

    Ok(())
}

/// Deploys the UKI to the EFI partition.
fn deploy_uki_to_efi(efi_device: &str, staged_uki: &Path) -> Result<()> {
    if !Path::new(efi_device).exists() {
        bail!("EFI device {} does not exist", efi_device);
    }

    let mount_point = "/run/mnt/efi";
    disk::mount_efi_partition(efi_device, mount_point)?;

    std::fs::create_dir_all(format!("{}/EFI/BOOT", mount_point))?;

    let uki_path = uki::get_uki_path(Path::new(mount_point))?;
    std::fs::copy(staged_uki, &uki_path)
        .with_context(|| format!("Failed to copy UKI to {}", uki_path.display()))?;

    sync();

    disk::try_unmount(mount_point);

    kmsg::info!("UKI deployed to EFI partition");

    Ok(())
}

/// Initializes the STATE partition with config and secrets.
fn init_state_partition(
    device: &str,
    config: &HostConfig,
    server_pki: &ServerPki,
    sb_hierarchy: Option<&KeyHierarchy>,
) -> Result<()> {
    kmsg::info!("Initializing STATE partition");

    let mount_point = "/run/mnt/state";

    std::fs::create_dir_all(mount_point)
        .with_context(|| format!("Failed to create mount point {}", mount_point))?;

    mount(device, mount_point, "btrfs", MountFlags::empty(), None)
        .context("Failed to mount STATE partition")?;

    let config_toml = sysconfig::serialize(config).context("Failed to serialize config")?;
    std::fs::write(format!("{}/config.toml", mount_point), config_toml)
        .context("Failed to write config.toml")?;

    let secrets_dir = format!("{}/secrets", mount_point);
    std::fs::create_dir_all(&secrets_dir).context("Failed to create secrets directory")?;

    std::fs::write(format!("{}/ca.crt", secrets_dir), &server_pki.ca_pem)
        .context("Failed to write CA certificate")?;
    std::fs::write(format!("{}/ca.key", secrets_dir), &server_pki.ca_key_pem)
        .context("Failed to write CA key")?;
    std::fs::write(
        format!("{}/server.crt", secrets_dir),
        &server_pki.server_cert_pem,
    )
    .context("Failed to write server certificate")?;
    std::fs::write(
        format!("{}/server.key", secrets_dir),
        &server_pki.server_key_pem,
    )
    .context("Failed to write server key")?;

    if let Some(hierarchy) = sb_hierarchy {
        save_key_hierarchy(hierarchy, &Path::new(&secrets_dir).join("secureboot"))
            .context("Failed to save Secure Boot keys")?;
    }

    sync();
    disk::try_unmount(mount_point);

    kmsg::info!("STATE partition initialized");
    Ok(())
}

/// Generates the CA, server, and signs the admin CSR.
fn generate_pki_and_sign_csr(
    csr_pem: &str,
    config: &HostConfig,
) -> Result<(InstallResult, ServerPki, HostConfig)> {
    kmsg::info!("Generating PKI and signing CSR");

    let (ca_key, ca_cert) =
        pki::generate_ca_certificate("Muak CA").context("Failed to generate CA certificate")?;

    let ca_cert_pem = ca_cert
        .to_pem(LineEnding::LF)
        .context("Failed to encode CA certificate")?;

    let ca_key_pem =
        pki::util::pkcs8_to_pem(ca_key.pkcs8_der()).context("Failed to encode CA key")?;

    let (server_key, server_cert) =
        pki::generate_server_certificate("muak-server", &ca_key, &ca_cert)
            .context("Failed to generate server certificate")?;

    let server_cert_pem = server_cert
        .to_pem(LineEnding::LF)
        .context("Failed to encode server certificate")?;

    let server_key_pem =
        pki::util::pkcs8_to_pem(server_key.pkcs8_der()).context("Failed to encode server key")?;

    let (admin_cert, admin_fingerprint) =
        pki::sign_csr(csr_pem, &ca_key_pem, &ca_cert).context("Failed to sign admin CSR")?;

    let admin_cert_pem = admin_cert
        .to_pem(LineEnding::LF)
        .context("Failed to encode admin certificate")?;

    kmsg::info!("Admin fingerprint: {}", admin_fingerprint);

    let mut config_with_auth = config.clone();
    config_with_auth.auth = AuthConfig {
        users: vec![AuthUser {
            fingerprint: admin_fingerprint,
            permissions: vec![Permission::Admin],
        }],
        revoked: vec![],
    };

    let client_result = InstallResult {
        ca_pem: ca_cert_pem.clone(),
        admin_cert_pem,
    };

    let server_pki = ServerPki {
        ca_pem: ca_cert_pem,
        ca_key_pem,
        server_cert_pem,
        server_key_pem,
    };

    Ok((client_result, server_pki, config_with_auth))
}
