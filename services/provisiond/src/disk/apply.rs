//! Generic executor that materializes a declarative plan onto a physical disk.

use std::fs::{File, OpenOptions};
use std::io::Seek as _;

use ::disk::doc::{self, Partition};
use ::disk::plan::{Plan, Size};
use ::disk::role::Role;
use anyhow::{Result, anyhow};
use parttable::gpt;
use parttable::gpt::layout::{Placement, PlacementRequest, Size as PlacementSize, Start};
use parttable::gpt::partition::Partition as GptPartition;
use parttable::gpt::table::Table;

use super::blkpg::{add_partition_blkpg, delete_all_partitions_blkpg};
use super::constants::SECTOR_SIZE;
use super::format::wait_for_device;
use super::gpt::{commit, format_partition_name, uuid_now, verify};
use super::wipe::wipe;

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
    /// Unique GPT partition GUID (hyphenated UUID string).
    pub(crate) partuuid: String,
    /// Placed size in bytes.
    pub(crate) size_bytes: u64,
    /// Resolved device path (e.g. `/dev/nvme0n1p1`).
    pub(crate) device: String,
}

impl AppliedPartition {
    /// Converts the applied partition into a disk document record.
    ///
    /// # Errors
    ///
    /// Returns an error when the partition carries no role (foreign
    /// partitions kept by the platform are not disk records).
    pub(crate) fn record(&self) -> Result<Partition> {
        let role = self.role.ok_or_else(|| {
            anyhow!(
                "applied partition '{}' has no role and cannot be recorded",
                self.name
            )
        })?;

        Ok(Partition {
            role,
            name: self.name.clone(),
            type_guid: self.type_guid.clone(),
            size: Size::Fixed(self.size_bytes),
            partuuid: Some(self.partuuid.clone()),
        })
    }
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
pub(crate) fn apply_plan(disk: &str, plan: &Plan) -> Result<AppliedPlan> {
    kmsg::info!(
        "Applying partition plan to {} ({} partitions, wipe={})",
        disk,
        plan.partitions.len(),
        plan.wipe
    );

    if plan.wipe {
        delete_all_partitions_blkpg(disk)?;
        wipe(disk)?;
    }

    let (mut file, disk_size) = open_disk_rw(disk)?;
    let sector_count = disk_size.checked_div(SECTOR_SIZE).unwrap_or(0);
    let mut gpt = load_table(&mut file, sector_count, plan.wipe)?;

    let created = place_all(&mut gpt, plan)?;

    commit(&mut file, &gpt, sector_count)?;
    drop(file);
    verify(disk);

    let mut applied = Vec::new();

    for (placement, role) in created {
        add_partition_blkpg(
            disk,
            placement.number,
            placement.partition.starting_lba,
            placement.partition.ending_lba,
        )?;

        let device = format_partition_name(disk, placement.number);
        wait_for_device(&device)?;
        applied.push(AppliedPartition {
            role,
            name: placement.partition.name.clone(),
            type_guid: doc::guid(&placement.partition.type_guid),
            partuuid: doc::guid(&placement.partition.unique_guid),
            size_bytes: partition_bytes(&placement.partition),
            device,
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

fn load_table(file: &mut File, sector_count: u64, wipe: bool) -> Result<Table> {
    if wipe {
        return Ok(Table::create(sector_count, SECTOR_SIZE, DISK_GUID)?);
    }

    match gpt::io::read(file) {
        Ok(table) => Ok(table),
        Err(error) => Err(anyhow!(
            "no partition table found; cannot append to a blank disk: {error}"
        )),
    }
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

fn open_disk_rw(disk: &str) -> Result<(File, u64)> {
    let mut file = OpenOptions::new().read(true).write(true).open(disk)?;
    let size = file.seek(std::io::SeekFrom::End(0))?;

    Ok((file, size))
}
