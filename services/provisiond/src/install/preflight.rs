//! Install preflight checks shared by the dry-run plan and the installer.

use std::path::Path;

use anyhow::Result;
use blockdev::types::Disk;

use super::secureboot;
use crate::check::Check;
use crate::disk;
use crate::disk::checks::{for_disk, running_system_disk, system_disk_in_use};
use crate::disk::target::Targets;

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
    if targets.is_split() {
        let data = disk::listed(inventory, targets.data(), "data")?;
        checks.extend(for_disk(data, force));
    }
    checks.push(system_disk_in_use(
        &targets.system,
        running_system_disk().as_deref(),
        allow_system_disk,
    ));
    checks.extend(secureboot::system_checks(secureboot));

    Ok(checks)
}

fn not_installed(force: bool) -> Check {
    if force || !Path::new(config::system::CONFIG_PATH).exists() {
        return Check::pass("not_installed", "System is not installed".to_owned());
    }

    Check::fail(
        "not_installed",
        "Cannot install from an already-installed system. Boot from live ISO or use --force."
            .to_owned(),
    )
}
