//! Install preflight checks shared by the dry-run plan and the installer.

use std::path::Path;

use super::checks::{for_device, running_system_disk, system_disk_in_use};
use super::{Targets, secureboot_check};
use crate::check::Check;

/// Runs every preflight check against the resolved install targets.
pub(crate) fn checks(
    targets: &Targets,
    force: bool,
    allow_system_disk: bool,
    secureboot: bool,
) -> Vec<Check> {
    let mut checks = vec![not_installed(force)];
    checks.extend(for_device(&targets.system, force));
    if targets.is_split() {
        checks.extend(for_device(targets.data(), force));
    }
    checks.push(system_disk_in_use(
        &targets.system,
        running_system_disk().as_deref(),
        allow_system_disk,
    ));
    checks.push(secureboot_check(secureboot));

    checks
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
