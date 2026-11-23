//! Primary/backup promotion and failover policy for the network supervisor.

#[cfg(feature = "ipv4")]
use alloc::vec::Vec;
use std::collections::{HashMap, HashSet};

#[cfg(feature = "ipv4")]
use addrconf::v4;
use config::system::network::InterfaceKind;
#[cfg(feature = "ipv4")]
use linkctl::interface::Command;
use netdev::Ops;
use netlink::link::ifname::Name;
use netlink::link::state::State;

use super::{NetworkSupervisor, owner, provision, view};

pub(super) fn is_primary_interface<N: Ops>(supervisor: &NetworkSupervisor<N>, name: &Name) -> bool {
    supervisor.state.primary.as_ref() == Some(name)
}

/// Returns true when the L3 owner's interface is failed or down.
pub(super) fn is_owner_failed<N: Ops>(supervisor: &NetworkSupervisor<N>) -> bool {
    let Some(owner) = owner::l3_owner(
        &supervisor.state,
        &supervisor.interfaces,
        &supervisor.owners,
    ) else {
        return false;
    };
    supervisor
        .interfaces
        .get(owner)
        .is_some_and(|handle| view::is_failed(&handle.snapshot_rx.borrow()))
}

pub(super) fn is_interface_configured<N: Ops>(
    supervisor: &NetworkSupervisor<N>,
    name: &Name,
) -> bool {
    supervisor
        .interfaces
        .get(name)
        .is_some_and(|handle| view::is_configured(&handle.snapshot_rx.borrow()))
}

pub(super) fn handle_primary_recovery<N: Ops>(supervisor: &mut NetworkSupervisor<N>, name: &Name) {
    kmsg::info!("Primary interface {} recovered", name);
    if let Err(e) = supervisor.state.ensure_operational() {
        kmsg::warn!("Unexpected state during primary recovery: {}", e);
    }
}

/// Restores a recovered backup as primary, unless the current primary is healthy.
pub(super) async fn handle_backup_recovery<N: Ops>(
    supervisor: &mut NetworkSupervisor<N>,
    recovered: &Name,
) {
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
        handle.snapshot_rx.borrow_and_update();
    }

    rebridge(supervisor, recovered).await;

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
    let Some(owner) = owner::l3_owner(
        &supervisor.state,
        &supervisor.interfaces,
        &supervisor.owners,
    ) else {
        return;
    };
    let healthy = supervisor.interfaces.get(owner).is_some_and(|handle| {
        let snapshot = handle.snapshot_rx.borrow();
        view::is_configured(&snapshot) && snapshot.link == State::Up
    });
    if !healthy {
        return;
    }
    if let Err(e) = supervisor.state.ensure_operational() {
        kmsg::warn!("Unexpected state after owner recovery: {e}");
    }
}

pub(super) async fn handle_primary_removed<N: Ops>(
    supervisor: &mut NetworkSupervisor<N>,
    name: &Name,
) {
    kmsg::info!("Primary interface {} removed", name);

    if let Some(new_primary) = first_eligible_backup(supervisor) {
        kmsg::info!("Promoting {} to primary", new_primary);
        supervisor.state.primary = Some(new_primary.clone());
        supervisor.state.backups.retain(|n| n != &new_primary);
        rebridge(supervisor, &new_primary).await;
    } else {
        kmsg::warn!("No eligible backup interface available");
        supervisor.state.primary = None;
        if let Err(e) = supervisor.state.ensure_degraded() {
            kmsg::warn!("Unexpected state during primary removal: {}", e);
        }
    }
}

fn is_eligible(
    physical: &HashSet<Name>,
    owners: &HashMap<Name, Name>,
    name: &Name,
    link: State,
) -> bool {
    physical.contains(name) && !owner::is_enslaved(owners, name) && link == State::Up
}

fn is_eligible_backup<N: Ops>(supervisor: &NetworkSupervisor<N>, name: &Name) -> bool {
    let Some(handle) = supervisor.interfaces.get(name) else {
        return false;
    };
    let link = handle.snapshot_rx.borrow().link;

    is_eligible(&supervisor.physical, &supervisor.owners, name, link)
}

fn first_eligible_backup<N: Ops>(supervisor: &NetworkSupervisor<N>) -> Option<Name> {
    supervisor
        .state
        .backups
        .iter()
        .find(|backup| is_eligible_backup(supervisor, backup))
        .cloned()
}

async fn try_failover_to_backup<N: Ops>(supervisor: &mut NetworkSupervisor<N>, failed: &Name) {
    let configured_backup = supervisor
        .state
        .backups
        .iter()
        .find(|backup| {
            is_interface_configured(supervisor, backup) && is_eligible_backup(supervisor, backup)
        })
        .cloned();
    let Some(new_primary) = configured_backup.or_else(|| first_eligible_backup(supervisor)) else {
        kmsg::info!("No backup available for failover");
        return;
    };

    kmsg::info!("Failing over from {} to {}", failed, new_primary);
    supervisor.state.backups.retain(|n| n != &new_primary);
    supervisor.state.backups.push(failed.clone());
    supervisor.state.primary = Some(new_primary.clone());

    #[cfg(feature = "ipv4")]
    if !rebridge(supervisor, &new_primary).await
        && !is_interface_configured(supervisor, &new_primary)
    {
        configure_promoted(supervisor, &new_primary).await;
    }
    #[cfg(not(feature = "ipv4"))]
    rebridge(supervisor, &new_primary).await;

    if let Err(e) = supervisor.state.ensure_operational() {
        kmsg::warn!("Unexpected state after failover to {}: {}", new_primary, e);
    }
}

// Starts DHCP on a promoted backup that has no bridge and no configuration yet.
#[cfg(feature = "ipv4")]
async fn configure_promoted<N: Ops>(supervisor: &NetworkSupervisor<N>, name: &Name) {
    let interfaces = supervisor.config.interfaces.clone();
    for iface_cfg in &interfaces {
        if iface_cfg.kind != InterfaceKind::Ethernet {
            continue;
        }
        let Ok(resolved) = provision::resolve_name(supervisor, &iface_cfg.name) else {
            continue;
        };
        if &resolved != name {
            continue;
        }
        let Some(ipv4) = iface_cfg.ipv4.as_ref() else {
            return;
        };
        if !ipv4.dhcp {
            return;
        }
        let config = v4::Config {
            addresses: Vec::new(),
            gateway: None,
            dns: supervisor.config.ipv4_dns().collect(),
            dhcp: true,
        };
        supervisor
            .send_to_interface(
                name,
                Command::Configure {
                    v4: Some(config),
                    v6: None,
                },
            )
            .await;
        return;
    }
}

// Points the bridge that owns the current primary at the new primary port.
async fn rebridge<N: Ops>(supervisor: &mut NetworkSupervisor<N>, port: &Name) -> bool {
    let mut has_bridge = false;
    let mut repointed = false;
    let interfaces = supervisor.config.interfaces.clone();
    for iface_cfg in &interfaces {
        if iface_cfg.kind != InterfaceKind::Bridge {
            continue;
        }
        let Some(bridge_cfg) = iface_cfg.bridge.clone() else {
            continue;
        };
        has_bridge = true;
        let Ok(port_name) = provision::bridge_port(supervisor, &bridge_cfg) else {
            continue;
        };
        if &port_name != port {
            continue;
        }
        if let Err(e) = provision::bridge::ensure(supervisor, &iface_cfg.name, &bridge_cfg).await {
            kmsg::warn!("Failed to re-point {} at {}: {}", iface_cfg.name, port, e);
        } else {
            repointed = true;
        }
    }
    if has_bridge && !repointed {
        kmsg::warn!("No bridge owns promoted port {port}");
    }
    repointed
}

#[cfg(test)]
mod tests {
    use super::*;

    fn name(value: &str) -> Name {
        Name::new(value).expect("valid name")
    }

    fn physical() -> HashSet<Name> {
        HashSet::from([name("eth0")])
    }

    fn owners_self() -> HashMap<Name, Name> {
        HashMap::from([(name("eth0"), name("eth0"))])
    }

    #[test]
    fn backup_eligibility_requires_physical_carrier() {
        // ARRANGE
        let eth0 = name("eth0");
        let physical = physical();
        let owners = owners_self();

        // ACT
        let up_eligible = is_eligible(&physical, &owners, &eth0, State::Up);
        let down_eligible = is_eligible(&physical, &owners, &eth0, State::Down);
        let virtual_eligible = is_eligible(&HashSet::new(), &owners, &eth0, State::Up);

        // ASSERT
        assert!(up_eligible, "a physical link with carrier is eligible");
        assert!(
            !down_eligible,
            "a link without carrier must not be a backup"
        );
        assert!(
            !virtual_eligible,
            "a link outside the physical set must not be a backup"
        );
    }

    #[test]
    fn backup_eligibility_rejects_enslaved_links() {
        // ARRANGE
        let eth0 = name("eth0");
        let physical = physical();
        let owners = HashMap::from([(eth0.clone(), name("br0")), (name("br0"), name("br0"))]);

        // ACT
        let eligible = is_eligible(&physical, &owners, &eth0, State::Up);

        // ASSERT
        assert!(!eligible, "a bridge port must not be a backup");
    }
}
