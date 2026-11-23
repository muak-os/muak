//! Command dispatch for one interface controller loop.

use core::fmt::Display;

use netdev::Ops;
use netlink::link::state::State as LinkState;

use super::{Command, Interface, Notice, Snapshot, event};
use crate::family::Family;
use crate::view;

pub(super) async fn run<N, V4, V6>(mut interface: Interface<N, V4, V6>)
where
    N: Ops,
    V4: Family<Config = addrconf::v4::Config, View = view::V4State>,
    V6: Family<Config = addrconf::v6::Config, View = view::V6State>,
{
    while let Some(selected) = event::select(&mut interface).await {
        dispatch(&mut interface, selected).await;
    }
}

async fn dispatch<N, V4, V6>(interface: &mut Interface<N, V4, V6>, selected: event::Selected)
where
    N: Ops,
    V4: Family<Config = addrconf::v4::Config, View = view::V4State>,
    V6: Family<Config = addrconf::v6::Config, View = view::V6State>,
{
    match selected {
        event::Selected::Command(command) => handle(interface, command).await,
        event::Selected::V4(event) => event::handle_v4(interface, event).await,
        event::Selected::V6(event) => event::handle_v6(interface, event).await,
    }
}

async fn handle<N, V4, V6>(interface: &mut Interface<N, V4, V6>, command: Command)
where
    N: Ops,
    V4: Family<Config = addrconf::v4::Config, View = view::V4State>,
    V6: Family<Config = addrconf::v6::Config, View = view::V6State>,
{
    match command {
        Command::Configure { v4, v6 } => {
            event::configure_v4(interface, v4).await;
            event::configure_v6(interface, v6).await;
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

fn stop_families<N, V4, V6>(interface: &mut Interface<N, V4, V6>)
where
    N: Ops,
    V4: Family<Config = addrconf::v4::Config, View = view::V4State>,
    V6: Family<Config = addrconf::v6::Config, View = view::V6State>,
{
    interface.v4.stop();
    interface.v6.stop();
}

fn forward<N, V4, V6>(interface: &mut Interface<N, V4, V6>, command: addrconf::Command)
where
    N: Ops,
    V4: Family<Config = addrconf::v4::Config, View = view::V4State>,
    V6: Family<Config = addrconf::v6::Config, View = view::V6State>,
{
    interface.v4.forward(command);
    interface.v6.forward(command);
}

async fn refresh_index<N, V4, V6>(interface: &mut Interface<N, V4, V6>)
where
    N: Ops,
    V4: Family<Config = addrconf::v4::Config, View = view::V4State>,
    V6: Family<Config = addrconf::v6::Config, View = view::V6State>,
{
    match interface.ops.ensure_up(interface.link.name.as_str()).await {
        Ok(index) => interface.link.index = index,
        Err(error) => warn_refresh(interface, &error),
    }
}

fn warn_refresh<N, V4, V6, E: Display>(interface: &Interface<N, V4, V6>, error: &E)
where
    N: Ops,
    V4: Family<Config = addrconf::v4::Config, View = view::V4State>,
    V6: Family<Config = addrconf::v6::Config, View = view::V6State>,
{
    if !interface.v4.is_active() && !interface.v6.is_active() {
        return;
    }
    let warning = alloc::format!("ensure link {} is up: {error}", interface.link.name);
    if interface.v4.is_active() {
        interface.v4.notify(Notice::Warning(warning.clone()));
    }
    if interface.v6.is_active() {
        interface.v6.notify(Notice::Warning(warning));
    }
}

pub(super) fn publish<N, V4, V6>(interface: &Interface<N, V4, V6>)
where
    N: Ops,
    V4: Family<Config = addrconf::v4::Config, View = view::V4State>,
    V6: Family<Config = addrconf::v6::Config, View = view::V6State>,
{
    let snapshot = Snapshot {
        link: interface.link.link_state,
        v4: interface.v4.view(),
        v4_state: interface.v4.state(),
        v6: interface.v6.view(),
        v6_state: interface.v6.state(),
    };
    drop(interface.snapshot_tx.send(snapshot));
}
