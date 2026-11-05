//! Volume pipeline: partitioning, LUKS formatting, mapping and btrfs creation.

use ::disk::role::Role;
use anyhow::{Context as _, Result};
use tokio::sync::mpsc;

use super::layout::plans_from_doc;
use super::send_progress;
use crate::disk;
use crate::ipc::proto::provision::InstallProgress;

// dm-crypt mapping name for the STATE volume.
const DM_STATE: &str = Role::State.dm_name();

// dm-crypt mapping name for the EPHEMERAL volume.
const DM_EPHEMERAL: &str = Role::Ephemeral.dm_name();

// dm-crypt mapping name for the DATA volume.
const DM_DATA: &str = Role::Data.dm_name();

pub(super) struct PartitionInfo {
    pub(super) efi: String,
    pub(super) state: String,
    pub(super) ephemeral: String,
    pub(super) data: String,
}

pub(super) struct Mapped {
    pub(super) state: String,
    pub(super) ephemeral: String,
    pub(super) data: String,
}

pub(super) async fn partition(
    targets: &disk::target::Targets,
    progress: &mpsc::Sender<InstallProgress>,
) -> Result<PartitionInfo> {
    send_progress(progress, &format!("Partitioning {}", targets.system)).await;

    let system = targets.system.clone();
    let data = targets.data().to_owned();
    let split = targets.is_split();

    tokio::task::spawn_blocking(move || partition_blocking(&system, &data, split)).await?
}

pub(super) async fn format_luks(
    partitions: &PartitionInfo,
    luks_key: &[u8],
    progress: &mpsc::Sender<InstallProgress>,
) -> Result<()> {
    send_progress(progress, "Formatting partitions...").await;

    disk::format::efi(&partitions.efi)?;

    let (state_result, ephemeral_result, data_result) = tokio::join!(
        luks_format(&partitions.state, luks_key, Role::State),
        luks_format(&partitions.ephemeral, luks_key, Role::Ephemeral),
        luks_format(&partitions.data, luks_key, Role::Data),
    );

    state_result?;
    ephemeral_result?;
    data_result?;

    Ok(())
}

pub(super) async fn open_luks(partitions: &PartitionInfo, luks_key: &[u8]) -> Result<Mapped> {
    let (state_result, ephemeral_result, data_result) = tokio::join!(
        luks_open(&partitions.state, luks_key, Role::State),
        luks_open(&partitions.ephemeral, luks_key, Role::Ephemeral),
        luks_open(&partitions.data, luks_key, Role::Data),
    );

    state_result?;
    ephemeral_result?;
    data_result?;

    Ok(Mapped {
        state: format!("/dev/mapper/{DM_STATE}"),
        ephemeral: format!("/dev/mapper/{DM_EPHEMERAL}"),
        data: format!("/dev/mapper/{DM_DATA}"),
    })
}

pub(super) async fn format_btrfs(mapped: &Mapped) -> Result<()> {
    let (state_result, ephemeral_result, data_result) = tokio::join!(
        btrfs_format(&mapped.state, Role::State),
        btrfs_format(&mapped.ephemeral, Role::Ephemeral),
        btrfs_format(&mapped.data, Role::Data),
    );

    state_result?;
    ephemeral_result?;
    data_result?;

    Ok(())
}

pub(super) fn close_luks() -> Result<()> {
    let mut failure = None;
    for name in [DM_DATA, DM_EPHEMERAL, DM_STATE] {
        if let Err(error) = luks2::close(name)
            && failure.is_none()
        {
            failure = Some((name, error));
        }
    }

    match failure {
        Some((name, error)) => {
            Err::<(), _>(error).with_context(|| format!("Failed to close LUKS mapping {name}"))
        }
        None => Ok(()),
    }
}

fn partition_blocking(system_disk: &str, data_disk: &str, split: bool) -> Result<PartitionInfo> {
    let shared_data = !split;
    let doc = disk::load_document()?;
    let (system_plan, data_plan) = plans_from_doc(&doc, shared_data)?;

    disk::validate::fits_plan(system_disk, &system_plan)?;
    if let Some(ref plan) = data_plan {
        disk::validate::fits_plan(data_disk, plan)?;
    }

    let system = disk::apply::plan(system_disk, &system_plan)?;
    let efi = system.role(Role::Esp)?.device.clone();
    let state = system.role(Role::State)?.device.clone();
    let ephemeral = system.role(Role::Ephemeral)?.device.clone();
    let data = match data_plan {
        Some(ref plan) => disk::apply::plan(data_disk, plan)?
            .role(Role::Data)?
            .device
            .clone(),
        None => system.role(Role::Data)?.device.clone(),
    };

    Ok(PartitionInfo {
        efi,
        state,
        ephemeral,
        data,
    })
}

async fn luks_format(device: &str, luks_key: &[u8], role: Role) -> Result<()> {
    let device = device.to_owned();
    let luks_key = luks_key.to_vec();

    flatten_join_result(tokio::task::spawn_blocking(move || {
        luks2::format(&device, &luks_key, role.gpt_name())
            .with_context(|| format!("Failed to LUKS format {role:?}"))
    }))
    .await
}

async fn luks_open(device: &str, luks_key: &[u8], role: Role) -> Result<()> {
    let device = device.to_owned();
    let name = role.dm_name();
    let luks_key = luks_key.to_vec();

    flatten_join_result(tokio::task::spawn_blocking(move || {
        luks2::open(&device, name, &luks_key)
            .with_context(|| format!("Failed to open LUKS {role:?}"))
    }))
    .await
}

async fn btrfs_format(device: &str, role: Role) -> Result<()> {
    let device = device.to_owned();

    flatten_join_result(tokio::task::spawn_blocking(move || {
        disk::format::btrfs(&device, role.gpt_name())
    }))
    .await
}

async fn flatten_join_result<T>(handle: tokio::task::JoinHandle<Result<T>>) -> Result<T> {
    handle
        .await
        .map_err(|e| anyhow::anyhow!("Task panicked: {e}"))?
}
