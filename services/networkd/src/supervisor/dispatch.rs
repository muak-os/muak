//! Netlink event consumer: dispatches received `NetworkEvent`s to supervisor state handlers.

use netlib::interface::Name;
use netlib::link::State;
use netlib::mac::format as format_mac;
use netlib::monitor::Event;
use netlib::netlink::Ops;

use super::NetworkSupervisor;
use super::failover;
use crate::interface::commands::Command;
use crate::interface::snapshot::Snapshot;
use crate::interface::state::Lifecycle;

/// Dispatches a netlink event to the appropriate handler.
pub(super) async fn handle_event<N: Ops>(supervisor: &mut NetworkSupervisor<N>, event: Event) {
    match event {
        Event::Up { name, index } => on_link_up(supervisor, name, index).await,
        Event::Down { name, index } => on_link_down(supervisor, name, index).await,
        Event::Added { name, index, mac } => {
            on_link_added(supervisor, name, index, mac);
        }
        Event::Deleted { name, index } => on_link_deleted(supervisor, name, index).await,
    }
}

async fn on_link_up<N: Ops>(supervisor: &mut NetworkSupervisor<N>, name: Name, index: u32) {
    kmsg::info!("Event: Link up {} (index {})", name, index);
    supervisor.send_to_interface(&name, Command::LinkUp).await;
    if failover::is_primary_interface(supervisor, &name) {
        failover::handle_primary_recovery(supervisor, &name);
        return;
    }
    if supervisor.state.backups.contains(&name) {
        failover::handle_backup_recovery(supervisor, &name);
    }
}

async fn on_link_down<N: Ops>(supervisor: &mut NetworkSupervisor<N>, name: Name, index: u32) {
    kmsg::info!("Event: Link down {} (index {})", name, index);
    supervisor.send_to_interface(&name, Command::LinkDown).await;
    if failover::is_primary_interface(supervisor, &name) {
        failover::handle_primary_failure(supervisor, &name);
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

    let snapshot = Snapshot {
        name: name.clone(),
        state: Lifecycle::Discovered,
        index,
        mac,
        link: State::Up,
        ip: None,
        lease: None,
        dhcp_state: None,
        ipv6: None,
        l3_owner: name.clone(),
    };
    supervisor.spawn_interface_actor(snapshot);

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

    supervisor.sync_and_publish();
}

async fn on_link_deleted<N: Ops>(supervisor: &mut NetworkSupervisor<N>, name: Name, index: u32) {
    kmsg::info!("Event: Link deleted {} (index {})", name, index);

    let Some(actor_handle) = supervisor.interfaces.remove(&name) else {
        return;
    };
    drop(actor_handle.cmd_tx.send(Command::Shutdown).await);

    if failover::is_primary_interface(supervisor, &name) {
        failover::handle_primary_removed(supervisor, &name);
    } else {
        supervisor.state.backups.retain(|n| n != &name);
    }

    supervisor.sync_and_publish();
}
