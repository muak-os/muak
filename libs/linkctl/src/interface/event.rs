//! Events an interface controller selects between and dispatches.

use core::task::Poll;
use std::time::Instant;

use addrconf::Event;
use netdev::Ops;

use super::{Interface, Notice, command, intent, wake};
use crate::family::Family;
use crate::view;

/// One input event the controller loop selected.
pub(super) enum Selected {
    /// Command from the supervisor.
    Command(super::Command),
    /// Event from the IPv4 method.
    V4(Event),
    /// Event from the IPv6 method.
    V6(Event),
}

enum Wait {
    Input(Selected),
    Closed,
    Wake,
}

pub(super) async fn select<N, V4, V6>(interface: &mut Interface<N, V4, V6>) -> Option<Selected>
where
    N: Ops,
    V4: Family<Config = addrconf::v4::Config, View = view::V4State>,
    V6: Family<Config = addrconf::v6::Config, View = view::V6State>,
{
    loop {
        match wait(interface).await {
            Wait::Input(selected) => return Some(selected),
            Wait::Closed => return None,
            Wait::Wake => {}
        }
    }
}

async fn wait<N, V4, V6>(interface: &mut Interface<N, V4, V6>) -> Wait
where
    N: Ops,
    V4: Family<Config = addrconf::v4::Config, View = view::V4State>,
    V6: Family<Config = addrconf::v6::Config, View = view::V6State>,
{
    let now = Instant::now();
    let deadline = wake::earliest_wake(interface, now);

    let mut command = core::pin::pin!(interface.command_rx.recv());
    let mut v4 = core::pin::pin!(interface.v4.next_event(now));
    let mut v6 = core::pin::pin!(interface.v6.next_event(now));
    let mut timer = core::pin::pin!(wake::wait_until(deadline));

    core::future::poll_fn(|cx| {
        if let Poll::Ready(command) = command.as_mut().poll(cx) {
            return Poll::Ready(match command {
                Some(command) => Wait::Input(Selected::Command(command)),
                None => Wait::Closed,
            });
        }
        if let Poll::Ready(event) = v4.as_mut().poll(cx) {
            return Poll::Ready(Wait::Input(Selected::V4(event)));
        }
        if let Poll::Ready(event) = v6.as_mut().poll(cx) {
            return Poll::Ready(Wait::Input(Selected::V6(event)));
        }
        if timer.as_mut().poll(cx).is_ready() {
            return Poll::Ready(Wait::Wake);
        }
        Poll::Pending
    })
    .await
}

pub(super) async fn handle_v4<N, V4, V6>(interface: &mut Interface<N, V4, V6>, event: Event)
where
    N: Ops,
    V4: Family<Config = addrconf::v4::Config, View = view::V4State>,
    V6: Family<Config = addrconf::v6::Config, View = view::V6State>,
{
    match event {
        Event::Intent(intent) => intent::apply(interface, intent).await,
        Event::State(state) => interface.v4.transition(state),
        Event::Warning(text) => interface.v4.notify(Notice::Warning(text)),
        Event::Failed(failure) => interface.v4.fail(failure.to_string()),
    }
    command::publish(interface);
}

pub(super) async fn handle_v6<N, V4, V6>(interface: &mut Interface<N, V4, V6>, event: Event)
where
    N: Ops,
    V4: Family<Config = addrconf::v4::Config, View = view::V4State>,
    V6: Family<Config = addrconf::v6::Config, View = view::V6State>,
{
    match event {
        Event::Intent(intent) => intent::apply(interface, intent).await,
        Event::State(state) => interface.v6.transition(state),
        Event::Warning(text) => interface.v6.notify(Notice::Warning(text)),
        Event::Failed(failure) => interface.v6.fail(failure.to_string()),
    }
    command::publish(interface);
}

pub(super) async fn configure_v4<N, V4, V6>(
    interface: &mut Interface<N, V4, V6>,
    config: Option<addrconf::v4::Config>,
) where
    N: Ops,
    V4: Family<Config = addrconf::v4::Config, View = view::V4State>,
    V6: Family<Config = addrconf::v6::Config, View = view::V6State>,
{
    let Some(config) = config else {
        return;
    };
    interface.v4.configure(config, &interface.link).await;
}

pub(super) async fn configure_v6<N, V4, V6>(
    interface: &mut Interface<N, V4, V6>,
    config: Option<addrconf::v6::Config>,
) where
    N: Ops,
    V4: Family<Config = addrconf::v4::Config, View = view::V4State>,
    V6: Family<Config = addrconf::v6::Config, View = view::V6State>,
{
    let Some(config) = config else {
        return;
    };
    interface.v6.configure(config, &interface.link).await;
}
