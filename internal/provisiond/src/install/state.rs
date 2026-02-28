//! STATE partition initialization: writes config, auth, PKI, and Secure Boot keys to disk.

use std::io::Write;
use std::os::unix::fs::OpenOptionsExt;
use std::path::Path;

use anyhow::{Context, Result};
use rustix::fs::sync;
use rustix::mount::{MountFlags, mount};
use sbolt::keys::{KeyHierarchy, save_key_hierarchy};
use sysconfig::{AuthConfig, HostConfig};

use super::pki::ServerPki;
use crate::disk;

/// Mount point for the STATE partition during provisioning.
const MOUNT_POINT: &str = "/run/mnt/state";

/// Mounts the STATE partition and writes all initial configuration and secrets to it.
pub fn init(
    device: &str,
    config: &HostConfig,
    auth_config: &AuthConfig,
    server_pki: &ServerPki,
    sb_hierarchy: Option<&KeyHierarchy>,
) -> Result<()> {
    std::fs::create_dir_all(MOUNT_POINT)
        .with_context(|| format!("Failed to create mount point {}", MOUNT_POINT))?;

    mount(device, MOUNT_POINT, "btrfs", MountFlags::empty(), None)
        .context("Failed to mount STATE partition")?;

    let config_toml = sysconfig::serialize(config).context("Failed to serialize config")?;
    std::fs::write(format!("{}/config.toml", MOUNT_POINT), config_toml)
        .context("Failed to write config.toml")?;

    let auth_toml =
        sysconfig::serialize_auth(auth_config).context("Failed to serialize auth config")?;
    std::fs::write(format!("{}/auth.toml", MOUNT_POINT), auth_toml)
        .context("Failed to write auth.toml")?;

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
