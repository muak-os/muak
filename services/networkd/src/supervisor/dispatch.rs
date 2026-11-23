//! Netlink event consumer: dispatches received netlink events to supervisor state handlers.

use linkctl::interface::Command;
use mac::format as format_mac;
use netdev::Ops;
use netdev::interface::Ethernet;
use netlink::link::ifname::Name;
use netlink::link::state::State;
use netlink_monitor::{Event, Failure};

use super::{NetworkSupervisor, carrier, dns, failover, owner, reconcile};

/// Dispatches a netlink event to the appropriate handler.
pub(super) async fn handle_event<N: Ops>(supervisor: &mut NetworkSupervisor<N>, event: Event) {
    match event {
        Event::Up { name, index } => on_link_up(supervisor, name, index).await,
        Event::Down { name, index } => on_link_down(supervisor, name, index).await,
        Event::Added { name, index, mac } => {
            on_link_added(supervisor, name, index, mac);
        }
        Event::Deleted { name, index } => on_link_deleted(supervisor, name, index).await,
        Event::Error(failure) => on_monitor_error(supervisor, failure).await,
    }
}

async fn on_link_up<N: Ops>(supervisor: &mut NetworkSupervisor<N>, name: Name, index: u32) {
    kmsg::info!("Event: Link up {} (index {})", name, index);
    supervisor.send_to_interface(&name, Command::LinkUp).await;
    if enslaved_by_snapshot(supervisor, &name) || master_index(supervisor, index).await.is_some() {
        carrier::refresh_owner(supervisor).await;
        return;
    }
    if failover::is_primary_interface(supervisor, &name) {
        failover::handle_primary_recovery(supervisor, &name);
        return;
    }
    if supervisor.state.backups.contains(&name) {
        failover::handle_backup_recovery(supervisor, &name).await;
    }
}

async fn on_link_down<N: Ops>(supervisor: &mut NetworkSupervisor<N>, name: Name, index: u32) {
    kmsg::info!("Event: Link down {} (index {})", name, index);
    supervisor.send_to_interface(&name, Command::LinkDown).await;
    if enslaved_by_snapshot(supervisor, &name) || master_index(supervisor, index).await.is_some() {
        carrier::refresh_owner(supervisor).await;
        return;
    }
    if failover::is_primary_interface(supervisor, &name) {
        failover::handle_primary_failure(supervisor, &name).await;
    }
}

fn on_link_added<N: Ops>(
    supervisor: &mut NetworkSupervisor<N>,
    name: Name,
    index: u32,
    mac: [u8; 6],
) {
    kmsg::info!(
        "Event: Link added {} (index {}, MAC {})",
        name,
        index,
        format_mac(&mac)
    );

    if supervisor.interfaces.contains_key(&name) {
        return;
    }

    supervisor.spawn_interface(Ethernet::new(name.clone(), index, mac, State::Up));

    if supervisor.state.primary.is_none() {
        kmsg::info!("Assigning {} as primary interface", name);
        supervisor.state.primary = Some(name);
    } else {
        kmsg::info!("Adding {} to backup interfaces", name);
        let pos = supervisor
            .state
            .backups
            .partition_point(|existing| existing < &name);
        supervisor.state.backups.insert(pos, name);
    }

    dns::flush(supervisor);
}

async fn on_link_deleted<N: Ops>(supervisor: &mut NetworkSupervisor<N>, name: Name, index: u32) {
    kmsg::info!("Event: Link deleted {} (index {})", name, index);

    let Some(handle) = supervisor.interfaces.remove(&name) else {
        return;
    };
    supervisor.owners.remove(&name);
    drop(handle.command_tx.send(Command::Shutdown).await);

    if failover::is_primary_interface(supervisor, &name) {
        failover::handle_primary_removed(supervisor, &name).await;
    } else {
        supervisor.state.backups.retain(|n| n != &name);
    }

    carrier::refresh_owner(supervisor).await;
    dns::flush(supervisor);
}

async fn on_monitor_error<N: Ops>(supervisor: &mut NetworkSupervisor<N>, failure: Failure) {
    kmsg::warn!("Network event monitor error: {failure}");
    reconcile::run(supervisor).await;
}

fn enslaved_by_snapshot<N: Ops>(supervisor: &NetworkSupervisor<N>, name: &Name) -> bool {
    supervisor.interfaces.contains_key(name) && owner::is_enslaved(&supervisor.owners, name)
}

async fn master_index<N: Ops>(supervisor: &NetworkSupervisor<N>, index: u32) -> Option<u32> {
    match supervisor.ops.master(index).await {
        Ok(master) => master,
        Err(e) => {
            kmsg::warn!("Failed to query master for link {index}: {e}");
            None
        }
    }
}
