//! Update preparation: builds the UKI assets and stages the new configuration.

use std::fs::{self, File};
use std::path::PathBuf;

use ::disk::role::Role;
use anyhow::{Context as _, Result, bail};
use blockdev::discover;
use config::system;
use rustix::fs::sync;
use tokio::sync::mpsc;
use wizard::artifact::Artifact;
use wizard::cache::Strategy;
use wizard::domain::profile::{CustomizationSpec, Profile};
use wizard::request::Request;

use super::document::BuildInputs;
use super::{UPDATE_DIR, failure, progress, secureboot, snapshot, stage};
use crate::disk::roles;
use crate::ipc::proto::provision::PrepareUpdateProgress;
use crate::{cache, disk, medium, streaming};

/// Prepares an update by staging the UKI components via the wizard.
pub(crate) async fn prepare(
    inputs: &BuildInputs,
    new_config: Option<Vec<u8>>,
    author: &str,
    strategy: Strategy,
    progress: mpsc::Sender<PrepareUpdateProgress>,
) -> Result<String> {
    match prepare_inner(inputs, new_config, author, strategy, progress).await {
        Ok(update_id) => Ok(update_id),
        Err(error) => {
            failure::record(&inputs.version, author, &error);
            Err(error)
        }
    }
}

async fn prepare_inner(
    inputs: &BuildInputs,
    new_config: Option<Vec<u8>>,
    author: &str,
    strategy: Strategy,
    progress: mpsc::Sender<PrepareUpdateProgress>,
) -> Result<String> {
    if strategy != Strategy::CacheOnly {
        cache::enforce();
    }
    let installed = system::config()?;
    let target = disk::target::resolve_system(&installed.disks).await?;
    verify_system_disk(&target)?;

    streaming::send_progress(
        &progress,
        PrepareUpdateProgress {
            message: format!("Pulling update for release: {}", inputs.version),
            ..Default::default()
        },
    )
    .await;

    let staging_dir = create_staging_dir()?;
    let install_profile = derive_install_profile(&inputs.extensions)?;

    let requested_secureboot = inputs.secureboot;
    if new_config.is_some() && requested_secureboot {
        let state = firmware::secureboot::system().unwrap_or_default();
        if !state.enabled && !state.setup_mode {
            bail!(
                "Firmware is not in Setup Mode, cannot enroll Secure Boot keys. \
                 Please reboot and reset your firmware to Setup Mode and try again."
            );
        }
    }

    let installed_secureboot = system::host()?.secureboot;

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
    let mut kernel_file = File::create(&kernel_path)
        .with_context(|| format!("create kernel file {}", kernel_path.display()))?;

    let initramfs_path = assets_dir.join("initramfs");
    let mut initramfs_file = File::create(&initramfs_path)
        .with_context(|| format!("create initramfs file {}", initramfs_path.display()))?;

    let request_version = inputs.version.clone();
    let request_registry = inputs.registry.clone();
    let reporter_progress = progress.clone();

    tokio::task::spawn_blocking(move || {
        let reporter = progress::Forwarder::new(reporter_progress);
        let pair = sb_hierarchy
            .as_ref()
            .map(|hierarchy| sbolt::keys::SigningPair {
                signer: &hierarchy.db.signer,
                certificate: &hierarchy.db.certificate,
            });

        let request = Request::new(request_version, &request_registry)
            .strategy(strategy)
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
        let request = request.progress(&reporter);

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

    let update_id = snapshot::generate_id();
    stage::esp_hash(&staging_dir);
    stage::config(&update_id, &inputs.version, new_config, author)?;

    sync();
    cache::enforce();

    Ok(update_id)
}

fn verify_system_disk(configured: &str) -> Result<()> {
    if configured.is_empty() {
        bail!("No system disk configured");
    }

    let state_device =
        roles::device(Role::State).context("STATE partition not found on any disk")?;
    let actual = discover::parent_disk(&state_device)
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

fn create_staging_dir() -> Result<PathBuf> {
    let dir = PathBuf::from(UPDATE_DIR);
    fs::create_dir_all(&dir).context("Failed to create update staging dir")?;

    Ok(dir)
}
