//! Installation workflow implementation for deploying Muak to a disk.

use std::io::Write;
use std::os::unix::fs::OpenOptionsExt;
use std::path::Path;

use anyhow::{Context, Result, bail};
use ring::rand::SecureRandom;
use rustix::fs::sync;
use rustix::mount::{MountFlags, mount};
use sbolt::keys::{KeyHierarchy, save_key_hierarchy};
use sysconfig::{AuthConfig, AuthUser, HostConfig, Permission};
use tokio::sync::mpsc;
use x509_cert::der::EncodePem;
use x509_cert::der::pem::LineEnding;

use crate::constants::{self, DM_DATA, DM_STATE, LUKS_KEY_SIZE};
use crate::disk;
use crate::services::proto::provision::{InstallProgress, InstallStep};
use crate::uki::{self, Uki};

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

/// Installs Muak to the specified disk with the given configuration.
pub fn install(
    disk_path: &str,
    force: bool,
    config: &HostConfig,
    admin_csr_pem: &str,
    progress: mpsc::Sender<InstallProgress>,
) -> Result<InstallResult> {
    println!("Starting installation to {}", disk_path);

    send_progress(
        &progress,
        InstallStep::Validating,
        &format!("Validating disk {}", disk_path),
    );
    validate_disk(disk_path, force)?;

    send_progress(
        &progress,
        InstallStep::GeneratingKeys,
        "Generating cryptographic keys",
    );
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
        println!("Secure Boot disabled, skipping key generation and enrollment");
        None
    };

    let luks_key = generate_luks_key()?;

    send_progress(
        &progress,
        InstallStep::GeneratingPki,
        "Generating PKI and signing CSR",
    );
    let (client_result, server_pki, auth_config) = generate_pki_and_sign_csr(admin_csr_pem)?;

    send_progress(
        &progress,
        InstallStep::PullingImage,
        &format!("Pulling installer image: {}", config.system.image),
    );
    let work_dir = Path::new(constants::INSTALL_DIR);
    let components = Uki::prepare(&config.system.image, &config.system.extensions, work_dir)?;
    let staged_uki = work_dir.join("staged.efi");

    send_progress(&progress, InstallStep::BuildingUki, "Building UKI");
    components.build(&staged_uki, Some(&luks_key))?;

    if let Some(ref hierarchy) = sb_hierarchy {
        send_progress(
            &progress,
            InstallStep::SigningUki,
            "Signing UKI for Secure Boot",
        );
        Uki::sign(&staged_uki, hierarchy)?;
    }

    if let Some(ref hierarchy) = sb_hierarchy {
        sbolt::efi::enroll_keys(hierarchy).context("Failed to enroll Secure Boot keys")?;
        println!("Secure Boot keys enrolled");
    }

    send_progress(
        &progress,
        InstallStep::Partitioning,
        &format!("Partitioning disk {}", disk_path),
    );
    disk::delete_all_partitions_blkpg(disk_path)?;
    disk::wipe_disk(disk_path)?;
    let (efi_part, state_part, data_part) = disk::create_partitions(disk_path)?;

    send_progress(
        &progress,
        InstallStep::Formatting,
        "Formatting partitions...",
    );
    disk::format_efi_partition(&efi_part)?;

    run_parallel(
        || luks2::format(&state_part, &luks_key, "STATE").context("Failed to LUKS format STATE"),
        || luks2::format(&data_part, &luks_key, "DATA").context("Failed to LUKS format DATA"),
    )?;

    run_parallel(
        || luks2::open(&state_part, DM_STATE, &luks_key).context("Failed to open LUKS STATE"),
        || luks2::open(&data_part, DM_DATA, &luks_key).context("Failed to open LUKS DATA"),
    )?;

    let dm_state = format!("/dev/mapper/{}", DM_STATE);
    let dm_data = format!("/dev/mapper/{}", DM_DATA);

    run_parallel(
        || disk::format_btrfs_partition(&dm_state, "STATE"),
        || disk::format_btrfs_partition(&dm_data, "DATA"),
    )?;

    send_progress(
        &progress,
        InstallStep::Deploying,
        "Deploying UKI to EFI partition",
    );
    deploy_uki_to_efi(&efi_part, &staged_uki)?;

    send_progress(
        &progress,
        InstallStep::Initializing,
        "Initializing STATE partition",
    );
    init_state_partition(
        &dm_state,
        &config,
        &auth_config,
        &server_pki,
        sb_hierarchy.as_ref(),
    )?;

    luks2::close(DM_STATE).context("Failed to close LUKS STATE mapping")?;
    luks2::close(DM_DATA).context("Failed to close LUKS DATA mapping")?;

    if let Err(e) = uki::cleanup_dir(work_dir) {
        eprintln!("Failed to cleanup work dir: {}", e);
    }

    sync();
    println!("Installation completed successfully!");

    send_progress(
        &progress,
        InstallStep::Completed,
        "Installation completed successfully",
    );

    Ok(client_result)
}

/// Validates the disk is suitable for installation.
fn validate_disk(disk_path: &str, force: bool) -> Result<()> {
    if !force && Path::new(sysconfig::CONFIG_PATH).exists() {
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

    println!("UKI deployed to EFI partition");

    Ok(())
}

/// Initializes the STATE partition with config, auth, and secrets.
fn init_state_partition(
    device: &str,
    config: &HostConfig,
    auth_config: &AuthConfig,
    server_pki: &ServerPki,
    sb_hierarchy: Option<&KeyHierarchy>,
) -> Result<()> {
    println!("Initializing STATE partition");

    let mount_point = "/run/mnt/state";

    std::fs::create_dir_all(mount_point)
        .with_context(|| format!("Failed to create mount point {}", mount_point))?;

    mount(device, mount_point, "btrfs", MountFlags::empty(), None)
        .context("Failed to mount STATE partition")?;

    let config = sysconfig::serialize(config).context("Failed to serialize config")?;
    std::fs::write(format!("{}/config.toml", mount_point), config)
        .context("Failed to write config.toml")?;

    let auth = sysconfig::serialize_auth(auth_config).context("Failed to serialize auth config")?;
    std::fs::write(format!("{}/auth.toml", mount_point), auth)
        .context("Failed to write auth.toml")?;

    let secrets_dir = format!("{}/secrets", mount_point);
    std::fs::create_dir_all(&secrets_dir).context("Failed to create secrets directory")?;

    std::fs::write(format!("{}/ca.crt", secrets_dir), &server_pki.ca_pem)
        .context("Failed to write CA certificate")?;
    std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(format!("{}/ca.key", secrets_dir))
        .and_then(|mut f| f.write_all(server_pki.ca_key_pem.as_bytes()))
        .context("Failed to write CA key")?;
    std::fs::write(
        format!("{}/server.crt", secrets_dir),
        &server_pki.server_cert_pem,
    )
    .context("Failed to write server certificate")?;
    std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(format!("{}/server.key", secrets_dir))
        .and_then(|mut f| f.write_all(server_pki.server_key_pem.as_bytes()))
        .context("Failed to write server key")?;

    if let Some(hierarchy) = sb_hierarchy {
        save_key_hierarchy(hierarchy, &Path::new(&secrets_dir).join("secureboot"))
            .context("Failed to save Secure Boot keys")?;
    }

    sync();
    disk::try_unmount(mount_point);

    println!("STATE partition initialized");
    Ok(())
}

/// Generates the CA, server, and signs the admin CSR.
fn generate_pki_and_sign_csr(csr_pem: &str) -> Result<(InstallResult, ServerPki, AuthConfig)> {
    println!("Generating PKI and signing CSR");

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

    let auth_config = AuthConfig {
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

    Ok((client_result, server_pki, auth_config))
}

/// Generates a random LUKS key.
fn generate_luks_key() -> Result<Vec<u8>> {
    let rng = ring::rand::SystemRandom::new();
    let mut key = vec![0u8; LUKS_KEY_SIZE];
    rng.fill(&mut key)
        .map_err(|_| anyhow::anyhow!("Failed to generate random LUKS key"))?;
    Ok(key)
}

/// Runs two fallible closures in parallel, returning the first error if any.
fn run_parallel<F, G>(f: F, g: G) -> Result<()>
where
    F: FnOnce() -> Result<()> + Send,
    G: FnOnce() -> Result<()> + Send,
{
    std::thread::scope(|s| {
        let a = s.spawn(f);
        let b = s.spawn(g);

        let ra = a.join().expect("thread panicked");
        let rb = b.join().expect("thread panicked");

        ra.and(rb)
    })
}

/// Sends a progress update
fn send_progress(tx: &mpsc::Sender<InstallProgress>, step: InstallStep, message: &str) {
    if tx
        .blocking_send(InstallProgress {
            step: step as i32,
            message: message.to_string(),
            ..Default::default()
        })
        .is_err()
    {
        eprintln!("Progress receiver dropped during: {}", message);
    }
}
