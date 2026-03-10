//! STATE partition initialization: writes config, auth, PKI, and Secure Boot keys to disk.

use std::io::Write;
use std::os::unix::fs::OpenOptionsExt;
use std::path::Path;

use anyhow::{Context, Result};
use config::{AUTH_EXTENSION, AuthConfig, CONFIG_EXTENSION, SystemConfig};
use rustix::fs::sync;
use rustix::mount::{MountFlags, mount};
use sbolt::keys::{KeyHierarchy, save_key_hierarchy};

use super::pki::ServerPki;
use crate::disk;
use crate::history::{self, ChangeKind};

/// Mount point for the STATE partition during provisioning.
const MOUNT_POINT: &str = "/run/mnt/state";

/// Mounts the STATE partition and writes all initial configuration and secrets to it.
pub fn init(
    device: &str,
    config: &SystemConfig,
    auth_config: &AuthConfig,
    server_pki: &ServerPki,
    sb_hierarchy: Option<&KeyHierarchy>,
) -> Result<()> {
    std::fs::create_dir_all(MOUNT_POINT)
        .with_context(|| format!("Failed to create mount point {}", MOUNT_POINT))?;

    mount(device, MOUNT_POINT, "btrfs", MountFlags::empty(), None)
        .context("Failed to mount STATE partition")?;

    let config_bytes = config::serialize(config).context("Failed to serialize config")?;
    std::fs::write(
        format!("{}/config.{}", MOUNT_POINT, CONFIG_EXTENSION),
        &config_bytes,
    )
    .context("Failed to write config")?;

    let auth_bytes =
        config::serialize_auth(auth_config).context("Failed to serialize auth config")?;
    std::fs::write(
        format!("{}/auth.{}", MOUNT_POINT, AUTH_EXTENSION),
        auth_bytes,
    )
    .context("Failed to write auth config")?;

    let secrets_dir = format!("{}/secrets", MOUNT_POINT);
    std::fs::create_dir_all(&secrets_dir).context("Failed to create secrets directory")?;

    std::fs::write(format!("{}/ca.crt", secrets_dir), &server_pki.ca_pem)
        .context("Failed to write CA certificate")?;

    write_secret(
        format!("{}/ca.key", secrets_dir),
        server_pki.ca_key_pem.as_bytes(),
    )
    .context("Failed to write CA key")?;

    std::fs::write(
        format!("{}/server.crt", secrets_dir),
        &server_pki.server_cert_pem,
    )
    .context("Failed to write server certificate")?;

    write_secret(
        format!("{}/server.key", secrets_dir),
        server_pki.server_key_pem.as_bytes(),
    )
    .context("Failed to write server key")?;

    if let Some(hierarchy) = sb_hierarchy {
        save_key_hierarchy(hierarchy, &Path::new(&secrets_dir).join("secureboot"))
            .context("Failed to save Secure Boot keys")?;
    }

    if let Err(e) = history::record("install", "system", ChangeKind::Install, &config_bytes) {
        eprintln!("Failed to record install history: {}", e);
    }

    sync();
    disk::try_unmount(MOUNT_POINT);

    Ok(())
}

/// Writes data to a file with restrictive 0o600 permissions.
fn write_secret(path: impl AsRef<Path>, data: &[u8]) -> Result<()> {
    std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(path)?
        .write_all(data)?;
    Ok(())
}
