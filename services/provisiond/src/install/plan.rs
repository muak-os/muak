//! Dry-run install plan: resolved targets, planned partitions and preflight checks.

use ::disk::plan::{Plan as LayoutPlan, Size};
use anyhow::{Context as _, Result};
use config::system;

use super::layout::plans_from_doc;
use crate::disk::preflight::{self, Check};
use crate::disk::{self, Targets};

/// One partition the install will place.
pub(crate) struct PlannedPartition {
    pub disk: String,
    pub role: String,
    pub name: String,
    pub size: Size,
}

/// Everything the install will resolve, create and destroy.
pub(crate) struct InstallPlan {
    pub system: Option<disk::DiskInfo>,
    pub data: Option<disk::DiskInfo>,
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
    let targets = disk::resolve_with(&config.disks, &inventory)?;
    let doc = disk::load_document()?;
    let (system_plan, data_plan) = plans_from_doc(&doc, !targets.is_split())?;

    let system = inventory_disk(&inventory, &targets.system, "system")?;
    let data = inventory_disk(&inventory, targets.data(), "data")?;

    let checked = targets.clone();
    let checks =
        tokio::task::spawn_blocking(move || preflight::checks(&checked, force, allow_system_disk))
            .await
            .context("preflight task")?;

    Ok(InstallPlan {
        system: Some(system),
        data: Some(data),
        shared_data: !targets.is_split(),
        wipe: doc.wipe(),
        partitions: planned(&targets, &system_plan, data_plan.as_ref()),
        checks,
    })
}

fn inventory_disk(
    inventory: &[disk::DiskInfo],
    device: &str,
    role: &str,
) -> Result<disk::DiskInfo> {
    inventory
        .iter()
        .find(|disk| disk.path == device)
        .cloned()
        .with_context(|| {
            format!(
                "resolved {role} disk '{device}' is missing from the disk inventory, confirm the disk is attached and is a whole disk"
            )
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

    fn info(path: &str) -> disk::DiskInfo {
        disk::DiskInfo {
            name: path.rsplit('/').next().unwrap_or_default().to_owned(),
            path: path.to_owned(),
            size_bytes: 1024,
            model: String::new(),
            serial: String::new(),
            wwid: String::new(),
            bus_path: String::new(),
            subsystem: String::new(),
            rotational: false,
            cdrom: false,
            removable: false,
            read_only: false,
            symlinks: Vec::new(),
            is_system: false,
            is_data: false,
            partitions: Vec::new(),
        }
    }

    #[test]
    fn listed_disks_are_found_by_path() {
        // ARRANGE
        let inventory = vec![info("/dev/sda")];

        // ACT
        let found = inventory_disk(&inventory, "/dev/sda", "system").expect("listed disk");

        // ASSERT
        assert_eq!(found.path, "/dev/sda");
    }

    #[test]
    fn resolved_devices_missing_from_the_inventory_are_refused() {
        // ARRANGE
        let inventory = vec![info("/dev/sda")];

        // ACT
        let error = inventory_disk(&inventory, "/dev/sdb", "system").unwrap_err();

        // ASSERT
        assert_eq!(
            error.to_string(),
            "resolved system disk '/dev/sdb' is missing from the disk inventory, confirm the disk is attached and is a whole disk"
        );
    }

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
