//! Install preflight checks for the dry-run plan and the installer.

use std::path::Path;

use anyhow::{Context as _, Result};
use blockdev::types::Disk;
use config::system;
use tokio::sync::mpsc;
use wizard::domain::profile::Profile;
use wizard::request::Request;

use super::{secureboot, send_progress};
use crate::check::Check;
use crate::disk;
use crate::disk::checks::{for_disk, running_system_disk, system_disk_in_use};
use crate::disk::target::Targets;
use crate::ipc::proto::provision::InstallProgress;

/// Runs every preflight check against the resolved install targets.
pub(crate) fn checks(
    targets: &Targets,
    inventory: &[Disk],
    force: bool,
    allow_system_disk: bool,
    secureboot: bool,
) -> Result<Vec<Check>> {
    let mut checks = vec![not_installed(force)];
    let system = disk::listed(inventory, &targets.system, "system")?;
    checks.extend(for_disk(system, force));
    checks.push(system_disk_in_use(
        &targets.system,
        running_system_disk(&blockdev::handle()).as_deref(),
        allow_system_disk,
    ));
    checks.extend(secureboot::system_checks(secureboot));

    Ok(checks)
}

/// Verifies the install catalog resolves from the configured registry.
pub(super) async fn verify_registry(
    install_profile: &Profile,
    config: &system::Config,
    progress: &mpsc::Sender<InstallProgress>,
) -> Result<()> {
    send_progress(progress, "Checking registry").await;

    let profile = install_profile.clone();
    let version = config.host.version.clone();
    let registry = config.host.registry.clone();

    tokio::task::spawn_blocking(move || -> Result<()> {
        let request = Request::new(version, &registry);
        wizard::resolver::plan(&request, &profile)
            .context("Failed to resolve the install catalog from the registry")?;

        Ok(())
    })
    .await
    .context("registry preflight task")?
}

fn not_installed(force: bool) -> Check {
    if force || !Path::new(system::CONFIG_PATH).exists() {
        return Check::pass("not_installed", "System is not installed".to_owned());
    }

    Check::fail(
        "not_installed",
        "Cannot install from an already-installed system. Boot from live ISO or use --force."
            .to_owned(),
    )
}
