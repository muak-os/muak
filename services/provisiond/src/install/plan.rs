//! Dry-run install plan: resolved targets, planned partitions and preflight checks.

use ::disk::plan::{Plan as LayoutPlan, Size};
use anyhow::{Context as _, Result};
use blockdev::types::Disk;
use config::system;

use super::layout::plans_from_doc;
use super::preflight;
use crate::check::Check;
use crate::disk;
use crate::disk::target::Targets;

/// One partition the install will place.
pub(crate) struct PlannedPartition {
    pub disk: String,
    pub role: String,
    pub name: String,
    pub size: Size,
}

/// Everything the install will resolve, create and destroy.
pub(crate) struct InstallPlan {
    pub system: Disk,
    pub data: Disk,
    pub shared_data: bool,
    pub wipe: bool,
    pub partitions: Vec<PlannedPartition>,
    pub checks: Vec<Check>,
}

/// Resolves `config` into an install plan without writing anything.
pub(crate) async fn resolve(
    config: &system::Config,
    force: bool,
    allow_system_disk: bool,
) -> Result<InstallPlan> {
    let inventory = disk::list_disks().await?;
    let targets = disk::target::resolve_with(&config.disks, &inventory)?;
    let doc = disk::load_document()?;
    let (system_plan, data_plan) = plans_from_doc(&doc, !targets.is_split())?;

    let system = disk::listed(&inventory, &targets.system, "system")?.clone();
    let data = disk::listed(&inventory, targets.data(), "data")?.clone();

    let checked = targets.clone();
    let secureboot = config.host.secureboot;
    let checks = tokio::task::spawn_blocking(move || {
        preflight::checks(&checked, &inventory, force, allow_system_disk, secureboot)
    })
    .await
    .context("preflight task")??;

    Ok(InstallPlan {
        system,
        data,
        shared_data: !targets.is_split(),
        wipe: doc.wipe(),
        partitions: planned(&targets, &system_plan, data_plan.as_ref()),
        checks,
    })
}

fn planned(
    targets: &Targets,
    system_plan: &LayoutPlan,
    data_plan: Option<&LayoutPlan>,
) -> Vec<PlannedPartition> {
    let mut partitions = partitions_for(&targets.system, system_plan);
    if let Some(plan) = data_plan {
        partitions.extend(partitions_for(targets.data(), plan));
    }

    partitions
}

fn partitions_for(device: &str, plan: &LayoutPlan) -> Vec<PlannedPartition> {
    plan.partitions
        .iter()
        .map(|spec| PlannedPartition {
            disk: device.to_owned(),
            role: spec
                .role
                .map_or_else(|| "foreign".to_owned(), |role| role.gpt_name().to_owned()),
            name: spec.name.clone(),
            size: spec.size,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use ::disk::layout::Layout;

    use super::*;

    #[test]
    fn partitions_carry_the_device_they_belong_to() {
        // ARRANGE
        let layout = Layout::Uefi.plan();

        // ACT
        let partitions = partitions_for("/dev/sda", &layout);

        // ASSERT
        assert!(!partitions.is_empty());
        assert!(partitions.iter().all(|part| part.disk == "/dev/sda"));
    }
}
