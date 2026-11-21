//! Primary/backup promotion and failover policy for the network supervisor.

use std::collections::HashMap;

use netdev::Ops;
use netlink::link::ifname::Name;
use netlink::link::state::State;

use super::NetworkSupervisor;
use super::snapshot::NetworkSnapshot;
use crate::interface::ActorHandle;
use crate::interface::commands::{ApplyMode, Command};
use crate::interface::lifecycle::Lifecycle;
use crate::interface::snapshot::Snapshot;

pub(super) fn is_primary_interface<N: Ops>(supervisor: &NetworkSupervisor<N>, name: &Name) -> bool {
    supervisor.state.primary.as_ref() == Some(name)
}

/// Returns the interface that owns L3, following the primary's bridge ownership.
pub(super) fn l3_owner<'a>(
    state: &'a NetworkSnapshot,
    interfaces: &'a HashMap<Name, ActorHandle>,
) -> Option<&'a Name> {
    let primary = state.primary.as_ref()?;
    let snapshot = interfaces.get(primary)?.state_rx.borrow();
    if snapshot.l3_owner == snapshot.name {
        return Some(primary);
    }

    Some(
        interfaces
            .get_key_value(&snapshot.l3_owner)
            .map_or(primary, |(name, _)| name),
    )
}

/// Returns true when the L3 owner's interface is failed or down.
pub(super) fn is_owner_failed<N: Ops>(supervisor: &NetworkSupervisor<N>) -> bool {
    let Some(owner) = l3_owner(&supervisor.state, &supervisor.interfaces) else {
        return false;
    };
    supervisor.interfaces.get(owner).is_some_and(|handle| {
        let snapshot = handle.state_rx.borrow();
        snapshot.state == Lifecycle::Failed || snapshot.link == State::Down
    })
}

pub(super) fn is_interface_configured<N: Ops>(
    supervisor: &NetworkSupervisor<N>,
    name: &Name,
) -> bool {
    supervisor
        .interfaces
        .get(name)
        .is_some_and(|handle| handle.state_rx.borrow().state == Lifecycle::Configured)
}

pub(super) fn handle_primary_recovery<N: Ops>(supervisor: &mut NetworkSupervisor<N>, name: &Name) {
    kmsg::info!("Primary interface {} recovered", name);
    if let Err(e) = supervisor.state.ensure_operational() {
        kmsg::warn!("Unexpected state during primary recovery: {}", e);
    }
}

/// Restores a recovered backup as primary, unless the current primary is healthy.
pub(super) fn handle_backup_recovery<N: Ops>(
    supervisor: &mut NetworkSupervisor<N>,
    recovered: &Name,
) {
    let owning = supervisor
        .interfaces
        .get(recovered)
        .is_some_and(|handle| owns_l3(&handle.state_rx.borrow()));
    if !owning {
        return;
    }

    let Some(current_primary) = supervisor.state.primary.clone() else {
        return;
    };

    if is_interface_configured(supervisor, &current_primary) {
        kmsg::info!(
            "Backup {} recovered; keeping configured primary {}",
            recovered,
            current_primary
        );
        return;
    }
    kmsg::info!(
        "Recovered interface {} restoring as primary (demoting {})",
        recovered,
        current_primary
    );

    supervisor.state.backups.retain(|n| n != recovered);
    supervisor.state.backups.push(current_primary);
    supervisor.state.primary = Some(recovered.clone());

    if let Some(handle) = supervisor.interfaces.get_mut(recovered) {
        handle.state_rx.borrow_and_update();
    }

    if let Err(e) = supervisor.state.ensure_operational() {
        kmsg::warn!("Unexpected state restoring primary {}: {}", recovered, e);
    }
}

pub(super) async fn handle_primary_failure<N: Ops>(
    supervisor: &mut NetworkSupervisor<N>,
    name: &Name,
) {
    kmsg::warn!("Primary interface {} failed", name);
    if let Err(e) = supervisor.state.ensure_degraded() {
        kmsg::warn!("Unexpected state during primary failure: {}", e);
    }

    try_failover_to_backup(supervisor, name).await;
}

/// Switches the primary when the elected primary's interface has exhausted its DHCP attempts.
pub(super) async fn switch_if_needed<N: Ops>(supervisor: &mut NetworkSupervisor<N>) {
    if !is_owner_failed(supervisor) {
        return;
    }
    let Some(primary) = supervisor.state.primary.clone() else {
        return;
    };
    handle_primary_failure(supervisor, &primary).await;
}

/// Restores the operational state when the L3 owner is configured and has carrier.
pub(super) fn recover_if_possible<N: Ops>(supervisor: &mut NetworkSupervisor<N>) {
    if supervisor.state.is_operational() {
        return;
    }
    let Some(owner) = l3_owner(&supervisor.state, &supervisor.interfaces) else {
        return;
    };
    let healthy = supervisor.interfaces.get(owner).is_some_and(|handle| {
        let snapshot = handle.state_rx.borrow();
        snapshot.state == Lifecycle::Configured && snapshot.link == State::Up
    });
    if !healthy {
        return;
    }
    if let Err(e) = supervisor.state.ensure_operational() {
        kmsg::warn!("Unexpected state after owner recovery: {e}");
    }
}

pub(super) fn handle_primary_removed<N: Ops>(supervisor: &mut NetworkSupervisor<N>, name: &Name) {
    kmsg::info!("Primary interface {} removed", name);

    if let Some(new_primary) = supervisor.state.backups.first().cloned() {
        kmsg::info!("Promoting {} to primary", new_primary);
        supervisor.state.primary = Some(new_primary.clone());
        supervisor.state.backups.retain(|n| n != &new_primary);
    } else {
        kmsg::warn!("No backup interfaces available");
        supervisor.state.primary = None;
        if let Err(e) = supervisor.state.ensure_degraded() {
            kmsg::warn!("Unexpected state during primary removal: {}", e);
        }
    }
}

fn owns_l3(snapshot: &Snapshot) -> bool {
    snapshot.l3_owner == snapshot.name
}

fn is_eligible_backup(handle: &ActorHandle) -> bool {
    let snapshot = handle.state_rx.borrow();

    owns_l3(&snapshot) && snapshot.link == State::Up
}

async fn try_failover_to_backup<N: Ops>(supervisor: &mut NetworkSupervisor<N>, failed: &Name) {
    let configured_backup = supervisor
        .state
        .backups
        .iter()
        .find(|backup| {
            is_interface_configured(supervisor, backup)
                && supervisor
                    .interfaces
                    .get(*backup)
                    .is_some_and(is_eligible_backup)
        })
        .cloned();
    let Some(new_primary) = configured_backup.or_else(|| {
        supervisor
            .state
            .backups
            .iter()
            .find(|backup| {
                supervisor
                    .interfaces
                    .get(*backup)
                    .is_some_and(is_eligible_backup)
            })
            .cloned()
    }) else {
        kmsg::info!("No backup available for failover");
        return;
    };

    kmsg::info!("Failing over from {} to {}", failed, new_primary);
    supervisor.state.backups.retain(|n| n != &new_primary);
    supervisor.state.backups.push(failed.clone());
    supervisor.state.primary = Some(new_primary.clone());

    if !is_interface_configured(supervisor, &new_primary) {
        supervisor
            .send_to_interface(
                &new_primary,
                Command::ConfigureDhcp {
                    mode: ApplyMode::Provision,
                },
            )
            .await;
    }

    if let Err(e) = supervisor.state.ensure_operational() {
        kmsg::warn!("Unexpected state after failover to {}: {}", new_primary, e);
    }
}

#[cfg(test)]
mod tests {
    use alloc::sync::Arc;

    use tokio::sync::{mpsc, watch};

    use super::*;
    use crate::interface::snapshot::Snapshot;

    fn handle(name: &str, link: State, l3_owner: &str) -> ActorHandle {
        let snapshot = Snapshot {
            name: Name::new(name).expect("valid name"),
            state: Lifecycle::Configured,
            index: 1,
            mac: [0; 6],
            link,
            ip: None,
            lease: None,
            dhcp_state: None,
            ipv6: None,
            l3_owner: Name::new(l3_owner).expect("valid name"),
        };
        let (cmd_tx, _cmd_rx) = mpsc::channel(1);
        let (_state_tx, state_rx) = watch::channel(Arc::new(snapshot));

        ActorHandle { cmd_tx, state_rx }
    }

    #[test]
    fn backup_eligibility_requires_own_l3_and_carrier() {
        // ARRANGE
        let slave = handle("eth1", State::Up, "br0");
        let down = handle("eth2", State::Down, "eth2");
        let ready = handle("eth3", State::Up, "eth3");

        // ACT
        let slave_eligible = is_eligible_backup(&slave);
        let down_eligible = is_eligible_backup(&down);
        let ready_eligible = is_eligible_backup(&ready);

        // ASSERT
        assert!(!slave_eligible, "a bridge slave must not be a backup");
        assert!(
            !down_eligible,
            "a link without carrier must not be a backup"
        );
        assert!(ready_eligible, "a configured link with carrier is eligible");
    }

    #[test]
    fn owns_l3_rejects_a_bridge_slave() {
        // ARRANGE
        let slave = handle("eth0", State::Up, "br0");
        let owner = handle("eth0", State::Down, "eth0");

        // ACT
        let slave_owns_l3 = owns_l3(&slave.state_rx.borrow());
        let owner_owns_l3 = owns_l3(&owner.state_rx.borrow());

        // ASSERT
        assert!(!slave_owns_l3, "a bridge slave must not own its L3 state");
        assert!(
            owner_owns_l3,
            "an interface named after its L3 owner owns L3 regardless of carrier"
        );
    }

    #[test]
    fn l3_owner_follows_bridge_ownership() {
        // ARRANGE
        let mut state = NetworkSnapshot::empty();
        state.primary = Some(Name::new("eth0").expect("valid name"));
        let interfaces = HashMap::from([
            (
                Name::new("eth0").expect("valid name"),
                handle("eth0", State::Up, "br0"),
            ),
            (
                Name::new("br0").expect("valid name"),
                handle("br0", State::Up, "br0"),
            ),
        ]);

        // ACT
        let owner = l3_owner(&state, &interfaces);

        // ASSERT
        assert_eq!(owner.map(Name::as_str), Some("br0"));
    }

    #[test]
    fn l3_owner_falls_back_to_primary_without_bridge_actor() {
        // ARRANGE
        let mut state = NetworkSnapshot::empty();
        state.primary = Some(Name::new("eth0").expect("valid name"));
        let interfaces = HashMap::from([(
            Name::new("eth0").expect("valid name"),
            handle("eth0", State::Up, "br0"),
        )]);

        // ACT
        let owner = l3_owner(&state, &interfaces);

        // ASSERT
        assert_eq!(owner.map(Name::as_str), Some("eth0"));
    }

    #[test]
    fn l3_owner_is_none_without_primary() {
        // ARRANGE
        let state = NetworkSnapshot::empty();
        let interfaces = HashMap::new();

        // ACT
        let owner = l3_owner(&state, &interfaces);

        // ASSERT
        assert_eq!(owner.map(Name::as_str), None);
    }
}
