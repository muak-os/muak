//! Update preparation, kexec execution, and post-boot validation.

mod commit;
pub mod kexec;
pub(crate) mod rollback;
pub(super) mod snapshot;
mod validation;

use std::fs::{self, File};
use std::path::{Path, PathBuf};

use anyhow::{Context as _, Result, bail};
use config::{CONFIG_PATH, SystemConfig};
use rollback::{ROLLBACKS_DIR, RollbackInfo};
use rustix::fs::sync;
use sbolt::efi::{secure_boot, setup_mode};
use sbolt::keys::hierarchy::Bundle;
use sbolt::keys::storage::{load_hierarchy, save_hierarchy};
use tokio::sync::mpsc;
use wizard::config::{Config, configure};
use wizard::profile::{CustomizationSpec, Profile};
use wizard::request::{Platform, Request};

use crate::constants::{SECRETS_DIR, UPDATE_DIR};
use crate::history::{self, ChangeKind};
use crate::ipc::proto::provision::PrepareUpdateProgress;
use crate::profile;
use crate::streaming;

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
    let rollback_path = Path::new(ROLLBACKS_DIR).join(format!("{update_id}.json"));
    if rollback_path.exists() {
        if let Ok(contents) = std::fs::read_to_string(&rollback_path)
            && let Ok(info) = serde_json::from_str::<RollbackInfo>(&contents)
        {
            return UpdateStatus::RolledBack(info.reason);
        }
        return UpdateStatus::RolledBack("Unknown error".to_owned());
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
    image: &str,
    extensions: &[String],
    new_config: Option<SystemConfig>,
    author: &str,
    progress: mpsc::Sender<PrepareUpdateProgress>,
) -> Result<String> {
    streaming::send_progress(
        &progress,
        PrepareUpdateProgress {
            message: format!("Pulling update image: {image}"),
            ..Default::default()
        },
    )
    .await;

    let staging_dir = create_staging_dir()?;
    let install_profile = derive_install_profile(extensions)?;

    if let Some(ref cfg) = new_config {
        let secure_boot_active = secure_boot().unwrap_or(false);
        let setup_mode_active = setup_mode().unwrap_or(false);
        if cfg.host.secureboot && !secure_boot_active && !setup_mode_active {
            bail!(
                "Firmware is not in Setup Mode, cannot enroll Secure Boot keys. \
                 Please reboot and reset your firmware to Setup Mode and try again."
            );
        }
    }

    let needs_sb =
        config::host().secureboot || new_config.as_ref().is_some_and(|cfg| cfg.host.secureboot);
    let sb_hierarchy = if needs_sb {
        Some(resolve_sb_hierarchy()?)
    } else {
        None
    };

    let (registry, installer, version) = image_parts(image)?;
    configure(Config {
        cache_dir: Some("/run/state/cache/koci".into()),
        installer: Some(installer),
        extension_registry: Some(registry),
    })
    .context("Failed to configure wizard")?;

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

    let sections = tokio::task::spawn_blocking(move || {
        let pair = sb_hierarchy
            .as_ref()
            .map(|hierarchy| sbolt::keys::SigningPair {
                signer: &hierarchy.db.signer,
                certificate: &hierarchy.db.certificate,
            });

        let request = Request::new(version, Platform::Metal)
            .uki(&mut uki_file)
            .context("set UKI target")?
            .kernel(&mut kernel_file)
            .context("set kernel target")?
            .initramfs(&mut initramfs_file)
            .context("set initramfs target")?;

        let request = match pair.as_ref() {
            Some(pair) => request.sign(pair),
            None => request,
        };

        request
            .build(&install_profile)
            .context("wizard update prepare")
            .map(|metadata| metadata.sections)
    })
    .await
    .context("wizard update task")??;

    let sections_path = assets_dir.join("sections.json");
    std::fs::write(
        &sections_path,
        serde_json::to_string(&sections).context("Failed to serialize UKI sections")?,
    )
    .with_context(|| format!("Failed to write sections to {}", sections_path.display()))?;

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
        update_config_image(&update_id, image, author)?;
    }

    sync();

    Ok(update_id)
}

fn derive_install_profile(extensions: &[String]) -> Result<Profile> {
    let booted = profile::load().context("failed to load booted profile")?;
    let customization =
        CustomizationSpec::new(extensions.to_vec()).context("invalid extensions")?;

    Ok(Profile::new(
        booted.overlay().cloned(),
        customization,
        booted.kernel().clone(),
    ))
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
    let installer = path;

    Ok((
        registry.to_owned(),
        installer.to_owned(),
        version.to_owned(),
    ))
}

/// Checks for a pending update snapshot and spawns validation in the background.
pub fn check_and_handle_pending_validation() -> Result<()> {
    let Some((update_id, snapshot_path)) = snapshot::find_pending()? else {
        return Ok(());
    };

    if !has_update_marker() {
        cleanup_stale();
        return Ok(());
    }

    tokio::spawn(async move {
        if let Err(e) = validation::validate(&update_id, &snapshot_path).await {
            kmsg::warn!("Pending validation failed: {}", e);
        }
    });

    Ok(())
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

pub(super) fn update_config_image(update_id: &str, image: &str, author: &str) -> Result<()> {
    let contents = std::fs::read_to_string(CONFIG_PATH).context("Failed to read config")?;
    let mut config: SystemConfig =
        config::parse_from_str(&contents).context("Failed to parse config")?;

    image.clone_into(&mut config.host.image);

    let updated_config = config::serialize(&config).context("Failed to serialize config")?;
    std::fs::write(CONFIG_PATH, &updated_config).context("Failed to write updated config")?;

    if let Err(e) = history::record(update_id, author, ChangeKind::Update, &updated_config) {
        eprintln!("Failed to record config history: {e}");
    }

    Ok(())
}

pub(super) fn update_config(
    update_id: &str,
    new_config: &SystemConfig,
    author: &str,
) -> Result<()> {
    let contents = std::fs::read_to_string(CONFIG_PATH).context("Failed to read config")?;
    let config: SystemConfig =
        config::parse_from_str(&contents).context("Failed to parse config")?;

    let mut merged = new_config.clone();
    merged.disk = config.disk.clone();

    let updated_config = config::serialize(&merged).context("Failed to serialize config")?;
    std::fs::write(CONFIG_PATH, &updated_config).context("Failed to write updated config")?;

    if let Err(e) = history::record(update_id, author, ChangeKind::Update, &updated_config) {
        eprintln!("Failed to record config history: {e}");
    }

    Ok(())
}

pub(super) fn resolve_sb_hierarchy() -> Result<Bundle> {
    let dir = Path::new(SECRETS_DIR).join("secureboot");
    if dir.exists() {
        load_hierarchy(&dir).context("Failed to load Secure Boot keys")
    } else {
        let keys = Bundle::generate("Muak").context("Failed to generate Secure Boot keys")?;
        save_hierarchy(&keys, &dir).context("Failed to save Secure Boot keys")?;

        Ok(keys)
    }
}

pub fn signal_cli_contact() {
    validation::signal_cli_contact();
}
