//! Generic executor that materializes a declarative plan onto a physical disk.

use std::fs::{File, OpenOptions};

use ::disk::plan;
use ::disk::plan::{Plan, Size};
use ::disk::role::Role;
use anyhow::{Result, anyhow, bail};
use blockdev::device;
use partdev::{blkpg, clear, table};
use parttable::gpt::layout::{Placement, PlacementRequest, Size as PlacementSize, Start};
use parttable::gpt::partition::Partition as GptPartition;
use parttable::gpt::table::Table;

use super::constants::SECTOR_SIZE;
use super::format::wait_for_device;
use super::gpt::{uuid_now, verify};

/// Deterministic disk GUID used so provisioned disks share a stable identifier.
const DISK_GUID: [u8; 16] = [0xff; 16];

/// One partition of an applied plan.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct AppliedPartition {
    /// Functional role; `None` for foreign partitions kept by the platform.
    pub(crate) role: Option<Role>,
    /// GPT partition name.
    pub(crate) name: String,
    /// GPT partition type GUID (hyphenated UUID string).
    pub(crate) type_guid: String,
    /// Placed size in bytes.
    pub(crate) size_bytes: u64,
    /// Resolved device path (e.g. `/dev/nvme0n1p1`).
    pub(crate) device: String,
}

/// Result of applying a plan to a disk.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct AppliedPlan {
    /// All applied partitions, preserved ones first, in placement order.
    pub(crate) partitions: Vec<AppliedPartition>,
}

impl AppliedPlan {
    /// Returns the applied partition with the given role.
    ///
    /// # Errors
    ///
    /// Returns an error when no applied partition carries the role.
    pub(crate) fn role(&self, role: Role) -> Result<&AppliedPartition> {
        self.partitions
            .iter()
            .find(|partition| partition.role == Some(role))
            .ok_or_else(|| anyhow!("plan for role '{role:?}' was not applied"))
    }
}

/// Applies a partition plan to `disk` and registers the partitions in the kernel.
///
/// # Errors
///
/// Returns an error when the plan cannot be materialized or devices fail to
/// appear.
pub(crate) fn plan(disk: &str, plan: &Plan) -> Result<AppliedPlan> {
    kmsg::info!(
        "Applying partition plan to {} ({} partitions, wipe={})",
        disk,
        plan.partitions.len(),
        plan.wipe
    );

    if plan.wipe {
        kmsg::info!("Removing all existing partitions from kernel for {}", disk);
        let removed = blkpg::delete_all(disk)?;
        super::log_removed(disk, &removed);
        clear::fast(disk)?;
    }

    let file = open_disk_rw(disk)?;
    let sector_count = table::sector_count(&file)?;
    let mut gpt = load_table(&file, sector_count, plan.wipe)?;

    let created = place_all(&mut gpt, plan)?;

    table::commit(&file, &gpt, sector_count)?;
    drop(file);
    verify(disk);

    let mut applied = Vec::new();

    for (placement, role) in created {
        kmsg::info!(
            "Adding partition {} using BLKPG ioctl (LBA {} to {})",
            placement.number,
            placement.partition.starting_lba,
            placement.partition.ending_lba
        );
        blkpg::add(
            disk,
            placement.number,
            placement.partition.starting_lba,
            placement.partition.ending_lba,
        )?;
        kmsg::info!("BLKPG: Successfully added partition {}", placement.number);

        let path = device::partition_path(disk, placement.number);
        wait_for_device(&path)?;
        applied.push(AppliedPartition {
            role,
            name: placement.partition.name.clone(),
            type_guid: plan::guid(&placement.partition.type_guid),
            size_bytes: partition_bytes(&placement.partition),
            device: path,
        });
    }

    kmsg::info!("Partition plan applied on {}", disk);

    Ok(AppliedPlan {
        partitions: applied,
    })
}

fn partition_bytes(partition: &GptPartition) -> u64 {
    partition
        .ending_lba
        .checked_sub(partition.starting_lba)
        .and_then(|spans| spans.checked_add(1))
        .and_then(|lbas| lbas.checked_mul(SECTOR_SIZE))
        .unwrap_or(0)
}

fn load_table(file: &File, sector_count: u64, wipe: bool) -> Result<Table> {
    if wipe {
        return Ok(Table::create(sector_count, SECTOR_SIZE, DISK_GUID)?);
    }

    let table = table::read(file).map_err(|error| {
        anyhow!("no partition table found; cannot append to a blank disk: {error}")
    })?;

    if table
        .partitions()
        .any(|(_, partition)| partition.name == Role::State.gpt_name())
    {
        bail!("disk already holds a Muak STATE partition; wipe it before applying a plan");
    }

    let device = Table::create(sector_count, SECTOR_SIZE, DISK_GUID)?;
    if table.last_usable_lba() > device.last_usable_lba() {
        bail!(
            "partition table claims usable LBA {} but the device only has {} sectors",
            table.last_usable_lba(),
            sector_count
        );
    }

    Ok(table)
}

fn place_all(gpt: &mut Table, plan: &Plan) -> Result<Vec<(Placement, Option<Role>)>> {
    let mut created = Vec::new();

    for spec in &plan.partitions {
        let request = PlacementRequest::new(
            spec.type_guid,
            *uuid::Uuid::new_v7(uuid_now()).as_bytes(),
            &spec.name,
            placement_size(spec.size),
        )
        .start(Start::AfterLastUsed);
        let placement = request.place(gpt, SECTOR_SIZE)?;
        created.push((placement, spec.role));
    }

    Ok(created)
}

fn placement_size(size: Size) -> PlacementSize {
    match size {
        Size::Fixed(bytes) => PlacementSize::Bytes(bytes),
        Size::Fill => PlacementSize::FillToLastUsable,
    }
}

fn open_disk_rw(disk: &str) -> Result<File> {
    Ok(OpenOptions::new().read(true).write(true).open(disk)?)
}

#[cfg(test)]
mod tests {
    use parttable::gpt::partition::{LINUX_FS_GUID, Partition};
    use tempfile::NamedTempFile;

    use super::*;

    // Creates a disk image of `sector_count` sectors holding a GPT with `names`.
    fn disk_with_table(sector_count: u64, names: &[&str]) -> NamedTempFile {
        let disk = NamedTempFile::new().expect("temp file");
        disk.as_file()
            .set_len(sector_count.saturating_mul(SECTOR_SIZE))
            .expect("resize");
        let mut table = Table::create(sector_count, SECTOR_SIZE, DISK_GUID).expect("table");

        for (index, name) in names.iter().enumerate() {
            let offset = u64::try_from(index).unwrap_or(0);
            let starting_lba = 2048_u64.saturating_add(offset.saturating_mul(4096));
            table.set_partition(
                u32::try_from(offset.saturating_add(1)).unwrap_or(0),
                Partition {
                    type_guid: LINUX_FS_GUID,
                    unique_guid: [u8::try_from(offset).unwrap_or(0); 16],
                    starting_lba,
                    ending_lba: starting_lba.saturating_add(4095),
                    attributes: 0,
                    name: (*name).to_owned(),
                },
            );
        }

        partdev::table::commit(disk.as_file(), &table, sector_count).expect("commit table");

        disk
    }

    fn open(disk: &NamedTempFile) -> File {
        OpenOptions::new()
            .read(true)
            .write(true)
            .open(disk.path())
            .expect("open disk")
    }

    #[test]
    fn load_table_rejects_geometry_larger_than_device() {
        // ARRANGE
        let disk = disk_with_table(0x0002_0000, &[]);
        disk.as_file()
            .set_len(32 * 1024 * 1024)
            .expect("truncate disk");
        let file = open(&disk);

        // ACT
        let result = load_table(&file, 0x0001_0000, false);

        // ASSERT
        assert!(
            result.is_err(),
            "a table authored for a larger disk must not be appended to"
        );
    }

    #[test]
    fn load_table_rejects_already_provisioned_disk() {
        // ARRANGE
        let disk = disk_with_table(0x0002_0000, &[Role::State.gpt_name()]);
        let file = open(&disk);

        // ACT
        let result = load_table(&file, 0x0002_0000, false);

        // ASSERT
        assert!(
            result.is_err(),
            "re-provisioning must not append a second Muak partition set"
        );
    }

    #[test]
    fn load_table_keeps_foreign_partitions() {
        // ARRANGE
        let disk = disk_with_table(0x0002_0000, &["Apple_APFS"]);
        let file = open(&disk);

        // ACT
        let table = load_table(&file, 0x0002_0000, false).expect("foreign table must load");

        // ASSERT
        assert!(
            table.is_partition_used(1),
            "preserved platform partitions must survive"
        );
    }
}
