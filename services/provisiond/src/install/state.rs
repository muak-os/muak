//! STATE partition initialization: writes config, auth, PKI, and Secure Boot keys to disk.

use std::io::Write as _;
use std::os::unix::fs::OpenOptionsExt as _;
use std::path::Path;

use anyhow::{Context as _, Result};
use config::{AUTH_EXTENSION, AuthConfig, CONFIG_EXTENSION, SystemConfig};
use rustix::fs::sync;
use rustix::mount::{MountFlags, mount};
use sbolt::keys::hierarchy;
use sbolt::keys::storage::save_hierarchy;

use super::pki::Server;
use crate::disk;
use crate::history::{self, ChangeKind};

/// Mount point for the STATE partition during provisioning.
const MOUNT_POINT: &str = "/run/mnt/state";

/// Mounts the STATE partition and writes all initial configuration and secrets to it.
pub fn init(
    device: &str,
    config: &SystemConfig,
    auth_config: &AuthConfig,
    server_pki: &Server,
    sb_hierarchy: Option<&hierarchy::Bundle>,
) -> Result<()> {
    std::fs::create_dir_all(MOUNT_POINT)
        .with_context(|| format!("Failed to create mount point {MOUNT_POINT}"))?;

    mount(device, MOUNT_POINT, "btrfs", MountFlags::empty(), None)
        .context("Failed to mount STATE partition")?;

    let config_bytes = config::serialize(config).context("Failed to serialize config")?;
    std::fs::write(
        format!("{MOUNT_POINT}/config.{CONFIG_EXTENSION}"),
        &config_bytes,
    )
    .context("Failed to write config")?;

    let auth_bytes =
        config::serialize_auth(auth_config).context("Failed to serialize auth config")?;
    std::fs::write(format!("{MOUNT_POINT}/auth.{AUTH_EXTENSION}"), auth_bytes)
        .context("Failed to write auth config")?;

    let secrets_dir = format!("{MOUNT_POINT}/secrets");
    std::fs::create_dir_all(&secrets_dir).context("Failed to create secrets directory")?;

    std::fs::write(format!("{secrets_dir}/ca.crt"), &server_pki.ca)
        .context("Failed to write CA certificate")?;

    write_secret(
        format!("{secrets_dir}/ca.key"),
        server_pki.ca_key.as_bytes(),
    )
    .context("Failed to write CA key")?;

    std::fs::write(format!("{secrets_dir}/server.crt"), &server_pki.cert)
        .context("Failed to write server certificate")?;

    write_secret(
        format!("{secrets_dir}/server.key"),
        server_pki.key.as_bytes(),
    )
    .context("Failed to write server key")?;

    if let Some(hierarchy) = sb_hierarchy {
        save_hierarchy(hierarchy, &Path::new(&secrets_dir).join("secureboot"))
            .context("Failed to save Secure Boot keys")?;
    }

    if let Err(e) = history::record("install", "system", ChangeKind::Install, &config_bytes) {
        eprintln!("Failed to record install history: {e}");
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
