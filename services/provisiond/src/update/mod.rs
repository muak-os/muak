//! Update preparation, kexec execution, and post-boot validation.

mod commit;
pub mod kexec;
mod readiness;
pub(crate) mod rollback;
mod secureboot;
pub(super) mod snapshot;
mod validation;

use std::fs::{self, File};
use std::path::{Path, PathBuf};

use anyhow::{Context as _, Result, bail};
use config::system::{self, CONFIG_PATH};
use rustix::fs::sync;
use tokio::sync::mpsc;
use wizard::artifact::Artifact;
use wizard::domain::profile::{CustomizationSpec, Profile};
use wizard::request::Request;

use crate::ipc::proto::provision::PrepareUpdateProgress;
use crate::journal::{self, ChangeKind, Entry};
use crate::{disk, medium, streaming};

/// Staging directory for update operations.
pub(crate) const UPDATE_DIR: &str = "/run/state/update";

/// Base directory for secrets on the mounted STATE partition.
pub(crate) const SECRETS_DIR: &str = "/run/state/secrets";

/// Status of a system update.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UpdateStatus {
    Unknown,
    Pending,
    Committed,
    RolledBack(String),
}

/// Returns the current status of a given update ID.
pub fn status(update_id: &str) -> UpdateStatus {
    if let Some(entry) = journal::find(update_id)
        && entry.kind() == &ChangeKind::Rollback
    {
        return UpdateStatus::RolledBack(entry.reason().unwrap_or("Unknown error").to_owned());
    }

    if snapshot::path(update_id).exists() {
        return UpdateStatus::Pending;
    }

    let cmdline = std::fs::read_to_string("/proc/cmdline").unwrap_or_default();
    if cmdline.contains(&format!("muak.update_id={update_id}")) {
        return UpdateStatus::Committed;
    }

    UpdateStatus::Unknown
}

/// Prepares an update by staging the UKI components via the wizard.
pub async fn prepare(
    registry: &str,
    version: &str,
    extensions: &[String],
    new_config: Option<system::Config>,
    author: &str,
    progress: mpsc::Sender<PrepareUpdateProgress>,
) -> Result<String> {
    crate::cache::enforce();
    let installed = config::system::config()?;
    let system = disk::resolve_system(&installed.disks).await?;
    verify_system_disk(&system)?;

    streaming::send_progress(
        &progress,
        PrepareUpdateProgress {
            message: format!("Pulling update for release: {version}"),
            ..Default::default()
        },
    )
    .await;

    let staging_dir = create_staging_dir()?;
    let install_profile = derive_install_profile(extensions)?;

    if let Some(ref cfg) = new_config {
        let state = firmware::secureboot::system().unwrap_or_default();
        let secure_boot_active = state.enabled;
        let setup_mode_active = state.setup_mode;
        if cfg.host.secureboot && !secure_boot_active && !setup_mode_active {
            bail!(
                "Firmware is not in Setup Mode, cannot enroll Secure Boot keys. \
                 Please reboot and reset your firmware to Setup Mode and try again."
            );
        }
    }

    let installed_secureboot = config::system::host()?.secureboot;
    let requested_secureboot = new_config
        .as_ref()
        .map_or(installed_secureboot, |cfg| cfg.host.secureboot);

    let sb_hierarchy = if installed_secureboot && !requested_secureboot {
        secureboot::hierarchy_for_removal()?
    } else if installed_secureboot || requested_secureboot {
        Some(secureboot::resolve_hierarchy()?)
    } else {
        None
    };

    let assets_dir = staging_dir.join("assets");
    fs::create_dir_all(&assets_dir)
        .with_context(|| format!("create assets dir {}", assets_dir.display()))?;
    let uki_path = assets_dir.join("uki.efi");
    let mut uki_file = File::create(&uki_path)
        .with_context(|| format!("create UKI file {}", uki_path.display()))?;

    let kernel_path = assets_dir.join("kernel");
    let mut kernel_file = fs::File::create(&kernel_path)
        .with_context(|| format!("create kernel file {}", kernel_path.display()))?;

    let initramfs_path = assets_dir.join("initramfs");
    let mut initramfs_file = File::create(&initramfs_path)
        .with_context(|| format!("create initramfs file {}", initramfs_path.display()))?;

    let request_version = version.to_owned();
    let request_registry = registry.to_owned();

    tokio::task::spawn_blocking(move || {
        let pair = sb_hierarchy
            .as_ref()
            .map(|hierarchy| sbolt::keys::SigningPair {
                signer: &hierarchy.db.signer,
                certificate: &hierarchy.db.certificate,
            });

        let request = Request::new(request_version, &request_registry)
            .artifact(Artifact::Uki, &mut uki_file)
            .context("set UKI target")?
            .artifact(Artifact::Kernel, &mut kernel_file)
            .context("set kernel target")?
            .artifact(Artifact::Initramfs, &mut initramfs_file)
            .context("set initramfs target")?;

        let request = match pair.as_ref() {
            Some(pair) => request.sign(pair),
            None => request,
        };

        request
            .build(&install_profile)
            .context("wizard update prepare")
    })
    .await
    .context("wizard update task")??;

    streaming::send_progress(
        &progress,
        PrepareUpdateProgress {
            message: "Finalizing update".to_owned(),
            ..Default::default()
        },
    )
    .await;

    let update_id = snapshot::create(&staging_dir)?;

    if let Some(cfg) = new_config {
        update_config(&update_id, &cfg, author)?;
    } else {
        update_config_version(&update_id, version, author)?;
    }

    sync();
    crate::cache::enforce();

    Ok(update_id)
}

fn verify_system_disk(configured: &str) -> Result<()> {
    if configured.is_empty() {
        bail!("No system disk configured");
    }

    let state_device = disk::find_partition_device(disk::Role::State)
        .context("STATE partition not found on any disk")?;
    let actual = disk::parent_disk(&state_device)
        .context("Failed to resolve the disk carrying the STATE partition")?;

    let configured = std::fs::canonicalize(configured)
        .with_context(|| format!("Configured system disk '{configured}' not found"))?;
    let configured_name = configured
        .file_name()
        .and_then(|name| name.to_str())
        .context("Invalid system disk path")?;

    if configured_name != actual {
        bail!(
            "Configured system disk '{}' does not match the disk carrying STATE ('/dev/{actual}')",
            configured.display()
        );
    }

    Ok(())
}

fn derive_install_profile(extensions: &[String]) -> Result<Profile> {
    let booted = medium::profile().context("failed to load booted profile")?;
    let customization =
        CustomizationSpec::new(extensions.to_vec()).context("invalid extensions")?;

    Ok(Profile::new(
        booted.overlay().cloned(),
        customization,
        booted.kernel().clone(),
    ))
}

/// Checks for a pending update snapshot and spawns validation in the background.
pub fn check_and_handle_pending_validation() -> Result<()> {
    let Some((update_id, snapshot_path)) = snapshot::find_pending()? else {
        return Ok(());
    };

    if !has_update_marker() {
        match snapshot::restore(&update_id, &snapshot_path, "Uncommitted update reverted") {
            Ok(true) => cleanup_stale(),
            Ok(false) => {
                kmsg::warn!("Revert of {update_id} untracked; snapshot kept for next-boot retry");
            }
            Err(e) => {
                kmsg::warn!("Failed to revert uncommitted update {}: {:#}", update_id, e);
            }
        }
        return Ok(());
    }

    tokio::spawn(async move {
        if let Err(e) = validation::validate(&update_id, &snapshot_path).await {
            kmsg::warn!("Pending validation failed: {:#}", e);
        }
    });

    Ok(())
}

/// Validates and returns the requested update target release.
///
/// # Errors
///
/// Returns an error when `requested` is empty or when it is a downgrade of
/// `current`.
pub fn resolve_target(current: &str, requested: &str) -> Result<String> {
    if requested.is_empty() {
        bail!("update target version must be specified");
    }

    config::version::check_no_downgrade(requested, current)?;

    Ok(requested.to_owned())
}

fn has_update_marker() -> bool {
    std::fs::read_to_string("/proc/cmdline")
        .unwrap_or_default()
        .contains("muak.update_id=")
}

fn cleanup_stale() {
    if let Err(e) = std::fs::remove_dir_all(Path::new(UPDATE_DIR)) {
        eprintln!("Failed to cleanup stale update dir: {e}");
    }
}

fn create_staging_dir() -> Result<PathBuf> {
    let dir = PathBuf::from(UPDATE_DIR);
    fs::create_dir_all(&dir).context("Failed to create update staging dir")?;

    Ok(dir)
}

pub(super) fn update_config_version(update_id: &str, version: &str, author: &str) -> Result<()> {
    let contents = std::fs::read_to_string(CONFIG_PATH).context("Failed to read config")?;
    let mut config: system::Config =
        config::system::parse_from_str(&contents).context("Failed to parse config")?;

    version.clone_into(&mut config.host.version);

    let updated_config =
        config::system::serialize(&config).context("Failed to serialize config")?;
    let entry = Entry::new(update_id, author, ChangeKind::Update);

    journal::append(&entry, &updated_config).context("Failed to append config journal entry")?;

    config::system::write_atomic(Path::new(CONFIG_PATH), updated_config.as_bytes())
        .context("Failed to write updated config")
}

pub(super) fn update_config(
    update_id: &str,
    new_config: &system::Config,
    author: &str,
) -> Result<()> {
    let contents = std::fs::read_to_string(CONFIG_PATH).context("Failed to read config")?;
    let config: system::Config =
        config::system::parse_from_str(&contents).context("Failed to parse config")?;

    let mut merged = new_config.clone();
    merged.disks.clone_from(&config.disks);

    let updated_config =
        config::system::serialize(&merged).context("Failed to serialize config")?;
    let entry = Entry::new(update_id, author, ChangeKind::Update);

    journal::append(&entry, &updated_config).context("Failed to append config journal entry")?;

    config::system::write_atomic(Path::new(CONFIG_PATH), updated_config.as_bytes())
        .context("Failed to write updated config")
}

pub fn signal_cli_contact() {
    validation::signal_cli_contact();
}

#[cfg(test)]
mod tests {
    use super::resolve_target;

    #[test]
    fn requested_release_is_guarded_and_returned() {
        // ARRANGE
        let current = "v1.2.0";

        // ACT
        let newer = resolve_target(current, "v1.3.0");
        let same = resolve_target(current, "v1.2.0");
        let older = resolve_target(current, "v1.1.0");
        let invalid = resolve_target(current, "garbage");

        // ASSERT
        newer.expect("newer release is valid");
        assert_eq!(same.expect("same release is valid"), "v1.2.0");
        older.expect_err("downgrade must fail");
        invalid.expect_err("invalid version must fail");
    }

    #[test]
    fn empty_release_is_rejected() {
        // ARRANGE
        let current = "v1.2.0";

        // ACT
        let error = resolve_target(current, "").expect_err("empty version must fail");

        // ASSERT
        assert!(error.to_string().contains("must be specified"));
    }
}
