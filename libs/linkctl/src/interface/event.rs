//! Events an interface controller selects between and dispatches.

use core::task::Poll;
use std::time::Instant;

use netdev::Ops;

#[cfg(feature = "ipv4")]
use super::v4;
#[cfg(feature = "ipv6")]
use super::v6;
use super::{Interface, command, intent, notice, wake};
use crate::provision;

/// One input event the controller loop selected.
pub(super) enum Selected {
    /// Command from the supervisor.
    Command(super::Command),
    /// Event from the IPv4 method.
    #[cfg(feature = "ipv4")]
    V4(provision::Event),
    /// Event from the IPv6 method.
    #[cfg(feature = "ipv6")]
    V6(provision::Event),
}

enum Wait {
    Input(Selected),
    Closed,
    Wake,
}

pub(super) async fn select<N: Ops>(interface: &mut Interface<N>) -> Option<Selected> {
    loop {
        match wait(interface).await {
            Wait::Input(selected) => return Some(selected),
            Wait::Closed => return None,
            Wait::Wake => {}
        }
    }
}

async fn wait<N: Ops>(interface: &mut Interface<N>) -> Wait {
    let now = Instant::now();
    let deadline = wake::earliest_wake(interface, now);

    let mut command = core::pin::pin!(interface.command_rx.recv());
    #[cfg(feature = "ipv4")]
    let mut v4 = core::pin::pin!(next_v4(&mut interface.v4, now));
    #[cfg(feature = "ipv6")]
    let mut v6 = core::pin::pin!(next_v6(&mut interface.v6, now));
    let mut timer = core::pin::pin!(wake::wait_until(deadline));

    core::future::poll_fn(|cx| {
        if let Poll::Ready(command) = command.as_mut().poll(cx) {
            return Poll::Ready(match command {
                Some(command) => Wait::Input(Selected::Command(command)),
                None => Wait::Closed,
            });
        }
        #[cfg(feature = "ipv4")]
        if let Poll::Ready(event) = v4.as_mut().poll(cx) {
            return Poll::Ready(Wait::Input(Selected::V4(event)));
        }
        #[cfg(feature = "ipv6")]
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

#[cfg(feature = "ipv4")]
pub(super) async fn handle_v4<N: Ops>(interface: &mut Interface<N>, event: provision::Event) {
    match event {
        provision::Event::Intent(intent) => intent::apply(interface, intent).await,
        provision::Event::State(state) => notice::transition_v4(interface, state),
        provision::Event::Warning(text) => {
            notice::notify_v4(interface, super::V4Notice::Warning(text));
        }
        provision::Event::Failed(failure) => notice::fail_v4(interface, failure.to_string()),
    }
    command::publish(interface);
}

#[cfg(feature = "ipv6")]
pub(super) async fn handle_v6<N: Ops>(interface: &mut Interface<N>, event: provision::Event) {
    match event {
        provision::Event::Intent(intent) => intent::apply(interface, intent).await,
        provision::Event::State(state) => notice::transition_v6(interface, state),
        provision::Event::Warning(text) => {
            notice::notify_v6(interface, super::V6Notice::Warning(text));
        }
        provision::Event::Failed(failure) => notice::fail_v6(interface, failure.to_string()),
    }
    command::publish(interface);
}

#[cfg(feature = "ipv4")]
pub(super) async fn configure_v4<N: Ops>(
    interface: &mut Interface<N>,
    config: Option<provision::v4::Config>,
) {
    let Some(config) = config else {
        return;
    };
    match v4::Method::from_config(config, &interface.link).await {
        Ok(mut method) => {
            method.on_command(provision::Command::Start);
            interface.v4 = Some(method);
            interface.v4_configured = false;
        }
        Err(failure) => {
            interface.v4 = None;
            notice::fail_v4(interface, failure.to_string());
        }
    }
}

#[cfg(feature = "ipv6")]
pub(super) fn configure_v6<N: Ops>(
    interface: &mut Interface<N>,
    config: Option<provision::v6::Config>,
) {
    let Some(config) = config else {
        return;
    };
    match v6::Method::from_config(&interface.link, config) {
        Ok(mut method) => {
            method.on_command(provision::Command::Start);
            interface.v6 = Some(method);
            interface.v6_configured = false;
        }
        Err(error) => {
            interface.v6 = None;
            notice::fail_v6(interface, error.to_string());
        }
    }
}

#[cfg(feature = "ipv4")]
async fn next_v4(method: &mut Option<v4::Method>, now: Instant) -> provision::Event {
    match method.as_mut() {
        Some(method) => method.next_event(now).await,
        None => core::future::pending().await,
    }
}

#[cfg(feature = "ipv6")]
async fn next_v6(method: &mut Option<v6::Method>, now: Instant) -> provision::Event {
    match method.as_mut() {
        Some(method) => method.next_event(now).await,
        None => core::future::pending().await,
    }
}
