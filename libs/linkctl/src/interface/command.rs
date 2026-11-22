//! Command dispatch for one interface controller loop.

use core::fmt::Display;

use netdev::Ops;
use netlink::link::state::State as LinkState;

#[cfg(feature = "ipv4")]
use super::V4Notice;
#[cfg(feature = "ipv6")]
use super::V6Notice;
use super::{Command, Interface, Snapshot, event, notice};

pub(super) async fn run<N: Ops>(mut interface: Interface<N>) {
    while let Some(selected) = event::select(&mut interface).await {
        dispatch(&mut interface, selected).await;
    }
}

async fn dispatch<N: Ops>(interface: &mut Interface<N>, selected: event::Selected) {
    match selected {
        event::Selected::Command(command) => handle(interface, command).await,
        #[cfg(feature = "ipv4")]
        event::Selected::V4(event) => event::handle_v4(interface, event).await,
        #[cfg(feature = "ipv6")]
        event::Selected::V6(event) => event::handle_v6(interface, event).await,
    }
}

async fn handle<N: Ops>(interface: &mut Interface<N>, command: Command) {
    match command {
        Command::Configure {
            #[cfg(feature = "ipv4")]
            v4,
            #[cfg(feature = "ipv6")]
            v6,
        } => {
            #[cfg(feature = "ipv4")]
            event::configure_v4(interface, v4).await;
            #[cfg(feature = "ipv6")]
            event::configure_v6(interface, v6);
            refresh_index(interface).await;
            publish(interface);
        }
        Command::Reconcile => forward(interface, addrconf::Command::Reconcile),
        Command::LinkUp => {
            interface.link.link_state = LinkState::Up;
            forward(interface, addrconf::Command::LinkUp);
            publish(interface);
        }
        Command::LinkDown => {
            interface.link.link_state = LinkState::Down;
            forward(interface, addrconf::Command::LinkDown);
            publish(interface);
        }
        Command::Shutdown => {
            stop_families(interface);
            publish(interface);
            interface.command_rx.close();
        }
    }
}

fn stop_families<N: Ops>(interface: &mut Interface<N>) {
    #[cfg(feature = "ipv4")]
    {
        notice::transition_v4(interface, addrconf::State::Stopped);
        interface.v4 = None;
    }
    #[cfg(feature = "ipv6")]
    {
        notice::transition_v6(interface, addrconf::State::Stopped);
        interface.v6 = None;
    }
}

fn forward<N: Ops>(interface: &mut Interface<N>, command: addrconf::Command) {
    #[cfg(feature = "ipv4")]
    if let Some(method) = interface.v4.as_mut() {
        method.on_command(command);
    }
    #[cfg(feature = "ipv6")]
    if let Some(method) = interface.v6.as_mut() {
        method.on_command(command);
    }
}

async fn refresh_index<N: Ops>(interface: &mut Interface<N>) {
    match interface.ops.ensure_up(interface.link.name.as_str()).await {
        Ok(index) => interface.link.index = index,
        Err(error) => warn_refresh(interface, &error),
    }
}

fn warn_refresh<N: Ops, E: Display>(interface: &Interface<N>, error: &E) {
    #[cfg(feature = "ipv4")]
    if interface.v4.is_some() {
        notice::notify_v4(
            interface,
            V4Notice::Warning(alloc::format!(
                "ensure link {} is up: {error}",
                interface.link.name
            )),
        );
    }
    #[cfg(feature = "ipv6")]
    if interface.v6.is_some() {
        notice::notify_v6(
            interface,
            V6Notice::Warning(alloc::format!(
                "ensure link {} is up: {error}",
                interface.link.name
            )),
        );
    }
}

pub(super) fn publish<N: Ops>(interface: &Interface<N>) {
    let snapshot = Snapshot {
        link: interface.link.link_state,
        #[cfg(feature = "ipv4")]
        v4: interface.v4.as_ref().and_then(super::v4::Method::view),
        #[cfg(feature = "ipv4")]
        v4_state: interface.v4_state,
        #[cfg(feature = "ipv6")]
        v6: interface.v6.as_ref().and_then(super::v6::Method::view),
        #[cfg(feature = "ipv6")]
        v6_state: interface.v6_state,
    };
    drop(interface.snapshot_tx.send(snapshot));
}
