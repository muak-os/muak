//! Bounded conversion between protobuf observations and the inventory core.

use anyhow::Result;
use coordination::inventory::{Snapshot, Vm, VmState};
use fleet_api::fleet::{NodeSnapshot, VmSummary};

use crate::model::identity;

pub(crate) fn from_wire(snapshot: NodeSnapshot) -> Result<Snapshot> {
    if snapshot.vms.len() > 1024 {
        anyhow::bail!("VM inventory limit exceeded");
    }
    let vms = snapshot
        .vms
        .into_iter()
        .map(|vm| {
            Ok(Vm {
                id: identity(&vm.vm_id)?,
                name: vm.name,
                state: from_state(vm.state),
            })
        })
        .collect::<Result<Vec<_>>>()?;
    let result = Snapshot {
        hostname: snapshot.hostname,
        vms,
    };
    result.validate(coordination::inventory::Limits::new(1, 1024, 255)?)?;
    Ok(result)
}

pub(crate) fn to_wire(snapshot: &Snapshot) -> NodeSnapshot {
    NodeSnapshot {
        hostname: snapshot.hostname.clone(),
        vms: snapshot
            .vms
            .iter()
            .map(|vm| VmSummary {
                vm_id: vm.id.to_string(),
                name: vm.name.clone(),
                state: to_state(vm.state),
            })
            .collect(),
    }
}

fn from_state(value: i32) -> VmState {
    match value {
        1 => VmState::Created,
        2 => VmState::Starting,
        3 => VmState::Running,
        4 => VmState::Stopping,
        5 => VmState::Stopped,
        6 => VmState::Failed,
        _ => VmState::Unknown,
    }
}
fn to_state(value: VmState) -> i32 {
    match value {
        VmState::Unknown => 0,
        VmState::Created => 1,
        VmState::Starting => 2,
        VmState::Running => 3,
        VmState::Stopping => 4,
        VmState::Stopped => 5,
        VmState::Failed => 6,
    }
}
