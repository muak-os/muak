//! Feedback from interface controllers and bridge topology.

use crate::{Supervisor, dns, failover, owner, provision};

/// Stores a controller snapshot and reacts to an owner change.
pub(crate) fn on_snapshot(
    supervisor: &mut Supervisor,
    name: &netmodel::Name,
    snapshot: &iface::Snapshot,
) {
    supervisor.snapshots.insert(name.clone(), snapshot.clone());
    if owner::owner(supervisor).is_none_or(|owner| owner != name) {
        return;
    }

    failover::switch_if_needed(supervisor);
    failover::recover_if_possible(supervisor);
    dns::flush(supervisor);
}

/// Registers a bridge that reported ready.
pub(crate) fn on_bridge_ready(
    supervisor: &mut Supervisor,
    name: &netmodel::Name,
    index: u32,
    mac: netmodel::Mac,
    state: netmodel::LinkState,
) {
    supervisor.known.insert(name.clone());
    supervisor.owners.insert(name.clone(), name.clone());
    supervisor.state.l3_owner = Some(name.clone());
    upsert_link(supervisor, name, index, mac, state);
    dns::flush(supervisor);
}

/// Records a port that joined a bridge.
pub(crate) fn on_enslaved(
    supervisor: &mut Supervisor,
    port: &netmodel::Name,
    master: &netmodel::Name,
) {
    supervisor.known.insert(master.clone());
    supervisor.owners.insert(port.clone(), master.clone());
    supervisor.enslaved.insert(port.clone(), master.clone());
    supervisor.state.l3_owner = Some(master.clone());
    provision::configure_master(supervisor, master);
    dns::flush(supervisor);
}

/// Records a port that left its bridge.
pub(crate) fn on_detached(supervisor: &mut Supervisor, port: &netmodel::Name) {
    supervisor.enslaved.remove(port);
    supervisor.owners.insert(port.clone(), port.clone());
    dns::flush(supervisor);
}

fn upsert_link(
    supervisor: &mut Supervisor,
    name: &netmodel::Name,
    index: u32,
    mac: netmodel::Mac,
    state: netmodel::LinkState,
) {
    if let Some(link) = supervisor.links.iter_mut().find(|link| &link.name == name) {
        link.admin = state.admin();
        link.carrier = state.carrier();
        return;
    }

    supervisor.links.push(netmodel::Link {
        name: name.clone(),
        index,
        mac,
        admin: state.admin(),
        carrier: state.carrier(),
        is_virtual: false,
        master: None,
    });
}
