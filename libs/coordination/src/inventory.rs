//! Bounded host observations, independent of a wire encoding.

use alloc::collections::BTreeSet;
use core::time::Duration;
use std::time::Instant;

use uuid::Uuid;

use crate::error::Failure;

/// Bounds on retained inventory. A transport must also bound frames before decoding.
#[derive(Debug, Clone, Copy)]
pub struct Limits {
    pub(crate) nodes: usize,
    pub(crate) vms_per_node: usize,
    pub(crate) name_bytes: usize,
}

impl Limits {
    /// Sets the maximum registered nodes, VMs per snapshot and bytes per name.
    /// Revoked registrations count toward the node limit.
    ///
    /// # Errors
    ///
    /// Returns [`Failure::InvalidLimits`] if any limit is zero.
    pub const fn new(
        nodes: usize,
        vms_per_node: usize,
        name_bytes: usize,
    ) -> Result<Self, Failure> {
        if nodes == 0 || vms_per_node == 0 || name_bytes == 0 {
            return Err(Failure::InvalidLimits);
        }
        Ok(Self {
            nodes,
            vms_per_node,
            name_bytes,
        })
    }
}

/// An observed lifecycle state, corresponding to the local VM API.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VmState {
    /// State not reported or not understood by the adapter.
    Unknown,
    /// Created but not yet started.
    Created,
    /// Starting the hypervisor.
    Starting,
    /// Running on the reporting host.
    Running,
    /// Stopping the hypervisor.
    Stopping,
    /// Stopped on the reporting host.
    Stopped,
    /// Local workload failure.
    Failed,
}

/// Minimal VM inventory; disk data and workload configuration are excluded.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Vm {
    /// Stable identity assigned by the local workload service.
    pub id: Uuid,
    /// Operator-visible name, bounded in bytes.
    pub name: String,
    /// Observed local lifecycle state.
    pub state: VmState,
}

/// A complete replacement for a node's previous observation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Snapshot {
    /// Operator-visible hostname, bounded in bytes.
    pub hostname: String,
    /// Complete, bounded VM inventory for this node.
    pub vms: Vec<Vm>,
}

impl Snapshot {
    pub(crate) fn validate(&self, limits: Limits) -> Result<(), Failure> {
        if self.vms.len() > limits.vms_per_node {
            return Err(Failure::SnapshotTooLarge);
        }
        validate_name(&self.hostname, limits.name_bytes)?;
        let mut identities = BTreeSet::new();
        for vm in &self.vms {
            validate_vm(vm, limits.name_bytes, &mut identities)?;
        }
        Ok(())
    }
}

/// The last accepted snapshot and its manager-local receive time.
#[derive(Debug)]
pub struct Observation {
    pub(crate) snapshot: Snapshot,
    pub(crate) sequence: u64,
    pub(crate) received_at: Instant,
}

impl Observation {
    /// Returns the last complete inventory, which may be stale.
    #[must_use = "callers need the retained snapshot"]
    pub const fn snapshot(&self) -> &Snapshot {
        &self.snapshot
    }

    /// Returns its sequence within the session that supplied it.
    #[must_use = "callers need the observation sequence"]
    pub const fn sequence(&self) -> u64 {
        self.sequence
    }

    /// Returns the receive time on the manager's monotonic clock.
    #[must_use = "callers need the manager-local receive time"]
    pub const fn received_at(&self) -> Instant {
        self.received_at
    }

    pub(crate) fn is_stale(&self, now: Instant, max_age: Duration) -> bool {
        now.checked_duration_since(self.received_at)
            .is_none_or(|age| age >= max_age)
    }
}

fn validate_name(value: &str, max_bytes: usize) -> Result<(), Failure> {
    if value.is_empty() || value.len() > max_bytes {
        return Err(Failure::InvalidName);
    }
    Ok(())
}

fn validate_vm(vm: &Vm, max_bytes: usize, identities: &mut BTreeSet<Uuid>) -> Result<(), Failure> {
    if vm.id.is_nil() {
        return Err(Failure::InvalidIdentity);
    }
    validate_name(&vm.name, max_bytes)?;
    if !identities.insert(vm.id) {
        return Err(Failure::DuplicateVm);
    }
    Ok(())
}
