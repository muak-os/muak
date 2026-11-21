//! Events an interface controller selects between and dispatches.

#[cfg(any(feature = "ipv4", feature = "ipv6"))]
use core::task::Poll;

use netdev::Ops;

#[cfg(feature = "ipv4")]
use super::v4;
#[cfg(feature = "ipv6")]
use super::v6;
use super::{Interface, intent};
#[cfg(any(feature = "ipv4", feature = "ipv6"))]
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

pub(super) async fn select<N: Ops>(interface: &mut Interface<N>) -> Option<Selected> {
    let mut command = core::pin::pin!(interface.command_rx.recv());
    #[cfg(feature = "ipv4")]
    let mut v4 = core::pin::pin!(next_v4(&mut interface.v4));
    #[cfg(feature = "ipv6")]
    let mut v6 = core::pin::pin!(next_v6(&mut interface.v6));

    core::future::poll_fn(|cx| {
        if let Poll::Ready(command) = command.as_mut().poll(cx) {
            return Poll::Ready(command.map(Selected::Command));
        }
        #[cfg(feature = "ipv4")]
        if let Poll::Ready(event) = v4.as_mut().poll(cx) {
            return Poll::Ready(Some(Selected::V4(event)));
        }
        #[cfg(feature = "ipv6")]
        if let Poll::Ready(event) = v6.as_mut().poll(cx) {
            return Poll::Ready(Some(Selected::V6(event)));
        }
        Poll::Pending
    })
    .await
}

#[cfg(feature = "ipv4")]
pub(super) async fn handle_v4<N: Ops>(interface: &mut Interface<N>, event: provision::Event) {
    match event {
        provision::Event::Intent(intent) => intent::apply(interface, intent).await,
        provision::Event::State(state) => interface.v4_state = state,
        provision::Event::Failed(failure) => {
            kmsg::warn!(
                "netstack: IPv4 method failed on {}: {failure}",
                interface.link.name
            );
            interface.v4_state = provision::State::Degraded;
        }
    }
    interface.publish();
}

#[cfg(feature = "ipv6")]
pub(super) async fn handle_v6<N: Ops>(interface: &mut Interface<N>, event: provision::Event) {
    match event {
        provision::Event::Intent(intent) => intent::apply(interface, intent).await,
        provision::Event::State(state) => interface.v6_state = state,
        provision::Event::Failed(failure) => {
            kmsg::warn!(
                "netstack: IPv6 method failed on {}: {failure}",
                interface.link.name
            );
            interface.v6_state = provision::State::Degraded;
        }
    }
    interface.publish();
}

#[cfg(feature = "ipv4")]
async fn next_v4(method: &mut Option<v4::Method>) -> provision::Event {
    match method.as_mut() {
        Some(method) => method.next_event().await,
        None => core::future::pending().await,
    }
}

#[cfg(feature = "ipv6")]
async fn next_v6(method: &mut Option<v6::Method>) -> provision::Event {
    match method.as_mut() {
        Some(method) => method.next_event().await,
        None => core::future::pending().await,
    }
}
