//! Link lifecycle events applied to the supervisor state.

use netmodel::{LinkEvent, Name};

use crate::actions::Action;
use crate::{State, Supervisor, dns, failover, owner, reconcile};

/// Applies one link lifecycle event.
pub(crate) fn apply(supervisor: &mut Supervisor, event: &LinkEvent) {
    match *event {
        LinkEvent::Up { name, .. } => on_up(supervisor, &name),
        LinkEvent::Down { name, .. } => on_down(supervisor, &name),
        LinkEvent::Added { name, index, mac } => on_added(supervisor, &name, index, mac),
        LinkEvent::Deleted { name, .. } => on_deleted(supervisor, &name),
        LinkEvent::Error(_) => reconcile::run(supervisor),
    }
}

fn on_up(supervisor: &mut Supervisor, name: &Name) {
    if matches!(
        supervisor.state.state,
        State::Initializing | State::Degraded
    ) && supervisor.state.primary.is_none()
    {
        supervisor.queue.push_back(Action::Discover);
        return;
    }
    if owner::is_enslaved(supervisor, name) {
        refresh_owner(supervisor);
        return;
    }
    if supervisor.state.primary.as_ref() == Some(name) {
        failover::handle_primary_recovery(supervisor, name);
        return;
    }
    if supervisor.state.backups.contains(name) {
        failover::handle_backup_recovery(supervisor, name);
    }
}

fn on_down(supervisor: &mut Supervisor, name: &Name) {
    if owner::is_enslaved(supervisor, name) {
        refresh_owner(supervisor);
        return;
    }
    if supervisor.state.primary.as_ref() == Some(name) {
        failover::handle_primary_failure(supervisor, name);
    }
}

fn on_added(supervisor: &mut Supervisor, _name: &Name, _index: u32, _mac: mac::Mac) {
    supervisor.queue.push_back(Action::Discover);
}

fn on_deleted(supervisor: &mut Supervisor, name: &Name) {
    let registered = supervisor.known.remove(name);
    supervisor.enslaved.remove(name);
    supervisor.physical.remove(name);
    supervisor.snapshots.remove(name);
    supervisor.owners.remove(name);
    supervisor.links.retain(|link| &link.name != name);
    if registered {
        supervisor
            .queue
            .push_back(Action::ShutdownInterface { name: *name });
    }

    if supervisor.state.primary.as_ref() == Some(name) {
        failover::handle_primary_removed(supervisor, name);
    } else {
        supervisor.state.backups.retain(|backup| backup != name);
    }

    refresh_owner(supervisor);
    dns::flush(supervisor);
    supervisor.queue.push_back(Action::Discover);
}

fn refresh_owner(supervisor: &mut Supervisor) {
    let Some(owner) = owner::owner(supervisor).copied() else {
        return;
    };
    if !supervisor.known.contains(&owner) {
        return;
    }
    supervisor
        .queue
        .push_back(Action::ReconcileInterface { name: owner });
}
