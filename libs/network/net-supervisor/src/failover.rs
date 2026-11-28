//! Primary/backup promotion and failover policy.

use netmodel::{InterfaceKind, Link, LinkState, Name};

use crate::{Supervisor, owner, provision, view};

/// Fails over when the current layer 3 owner reported a failure.
pub(crate) fn switch_if_needed(supervisor: &mut Supervisor) {
    if !is_owner_failed(supervisor) {
        return;
    }
    let Some(primary) = supervisor.state.primary else {
        return;
    };
    handle_primary_failure(supervisor, &primary);
}

/// Restores the operational state when the owner is configured and has carrier.
pub(crate) fn recover_if_possible(supervisor: &mut Supervisor) {
    if supervisor.state.is_operational() {
        return;
    }
    let Some(owner) = owner::owner(supervisor).copied() else {
        return;
    };
    let healthy = supervisor
        .snapshots
        .get(&owner)
        .is_some_and(|snapshot| view::is_configured(snapshot) && snapshot.link.has_carrier());
    if healthy {
        supervisor.state.ensure_operational();
    }
}

/// Marks the network degraded and promotes an eligible backup.
pub(crate) fn handle_primary_failure(supervisor: &mut Supervisor, name: &Name) {
    supervisor.state.ensure_degraded();
    try_failover_to_backup(supervisor, name);
}

/// Keeps the network operational when the primary reports carrier again.
pub(crate) fn handle_primary_recovery(supervisor: &mut Supervisor, _name: &Name) {
    supervisor.state.ensure_operational();
}

/// Restores a recovered backup as primary when the current primary is not configured.
pub(crate) fn handle_backup_recovery(supervisor: &mut Supervisor, recovered: &Name) {
    let Some(current) = supervisor.state.primary else {
        return;
    };
    if is_configured(supervisor, &current) {
        return;
    }

    supervisor.state.backups.retain(|name| name != recovered);
    supervisor.state.backups.push(current);
    supervisor.state.primary = Some(*recovered);
    rebridge(supervisor, recovered);
    supervisor.state.ensure_operational();
}

/// Promotes the first eligible backup when the primary interface disappears.
pub(crate) fn handle_primary_removed(supervisor: &mut Supervisor, _name: &Name) {
    if let Some(backup) = first_eligible_backup(supervisor) {
        supervisor.state.primary = Some(backup);
        supervisor.state.backups.retain(|name| name != &backup);
        rebridge(supervisor, &backup);
        return;
    }

    supervisor.state.primary = None;
    supervisor.state.ensure_degraded();
}

fn try_failover_to_backup(supervisor: &mut Supervisor, failed: &Name) {
    let configured = supervisor
        .state
        .backups
        .iter()
        .find(|backup| is_configured(supervisor, backup) && is_eligible(supervisor, backup))
        .copied();
    let Some(new_primary) = configured.or_else(|| first_eligible_backup(supervisor)) else {
        return;
    };

    supervisor.state.backups.retain(|name| name != &new_primary);
    supervisor.state.backups.push(*failed);
    supervisor.state.primary = Some(new_primary);

    if !rebridge(supervisor, &new_primary) && !is_configured(supervisor, &new_primary) {
        configure_promoted(supervisor, &new_primary);
    }
    supervisor.state.ensure_operational();
}

fn first_eligible_backup(supervisor: &Supervisor) -> Option<Name> {
    supervisor
        .state
        .backups
        .iter()
        .find(|backup| is_eligible(supervisor, backup))
        .copied()
}

fn is_eligible(supervisor: &Supervisor, name: &Name) -> bool {
    supervisor.physical.contains(name)
        && !owner::is_enslaved(supervisor, name)
        && link_state(supervisor, name).is_some_and(LinkState::has_carrier)
}

fn link_state(supervisor: &Supervisor, name: &Name) -> Option<LinkState> {
    supervisor
        .snapshots
        .get(name)
        .map(|snapshot| snapshot.link)
        .or_else(|| {
            supervisor
                .links
                .iter()
                .find(|link| &link.name == name)
                .map(Link::state)
        })
}

fn is_configured(supervisor: &Supervisor, name: &Name) -> bool {
    supervisor
        .snapshots
        .get(name)
        .is_some_and(view::is_configured)
}

fn is_owner_failed(supervisor: &Supervisor) -> bool {
    owner::owner(supervisor)
        .and_then(|owner| supervisor.snapshots.get(owner))
        .is_some_and(view::is_failed)
}

fn rebridge(supervisor: &mut Supervisor, port: &Name) -> bool {
    let bridges: Vec<usize> = supervisor
        .config
        .interfaces
        .iter()
        .enumerate()
        .filter(|&(_, config)| config.kind == InterfaceKind::Bridge)
        .map(|(index, _)| index)
        .collect();
    let mut repointed = false;
    for index in bridges {
        repointed |= repoint_bridge(supervisor, index, port);
    }

    repointed
}

fn repoint_bridge(supervisor: &mut Supervisor, index: usize, port: &Name) -> bool {
    if provision::bridge_port(supervisor, index).as_ref() != Some(port) {
        return false;
    }

    provision::queue_bridge(supervisor, index)
}

fn configure_promoted(supervisor: &mut Supervisor, name: &Name) {
    let Some(index) = promoted_config(supervisor, name) else {
        return;
    };
    let Some(config) = supervisor.config.interfaces.get(index) else {
        return;
    };
    if config.ipv4.as_ref().is_some_and(|ipv4| ipv4.dhcp) {
        provision::queue_configure(supervisor, index, *name);
    }
}

fn promoted_config(supervisor: &Supervisor, name: &Name) -> Option<usize> {
    supervisor
        .config
        .interfaces
        .iter()
        .enumerate()
        .find(|&(_, config)| {
            config.kind == InterfaceKind::Ethernet
                && provision::resolve_name(supervisor, &config.name).as_ref() == Some(name)
        })
        .map(|(index, _)| index)
}
