//! Supervisor event selection: waits for the next command, monitor event, primary change, or reconcile tick.

use alloc::sync::Arc;

use netlink_monitor::Event;
use tokio::sync::{mpsc, watch};
use tokio::time::Interval;

use super::commands::SupervisorCommand;
use crate::interface::snapshot::Snapshot;

/// A single event selected from the supervisor's input sources.
pub(super) enum SupervisorEvent {
    Command(SupervisorCommand),
    Netlink(Event),
    PrimaryChanged,
    ReconcileTick,
}

/// Waits for the next command, netlink event, primary snapshot change, or reconcile tick.
pub(super) async fn wait(
    cmd_rx: &mut mpsc::Receiver<SupervisorCommand>,
    event_rx: &mut mpsc::Receiver<Event>,
    primary_snap_rx: &mut Option<watch::Receiver<Arc<Snapshot>>>,
    reconcile: &mut Interval,
) -> Option<SupervisorEvent> {
    let mut cmd_fut = core::pin::pin!(cmd_rx.recv());
    let mut event_fut = core::pin::pin!(event_rx.recv());
    let mut snap_fut = core::pin::pin!(async {
        match primary_snap_rx.as_mut() {
            Some(rx) => rx.changed().await,
            None => core::future::pending().await,
        }
    });
    let mut tick_fut = core::pin::pin!(reconcile.tick());

    core::future::poll_fn(|cx| {
        if let core::task::Poll::Ready(Some(cmd)) = cmd_fut.as_mut().poll(cx) {
            return core::task::Poll::Ready(Some(SupervisorEvent::Command(cmd)));
        }
        if let core::task::Poll::Ready(Some(event)) = event_fut.as_mut().poll(cx) {
            return core::task::Poll::Ready(Some(SupervisorEvent::Netlink(event)));
        }
        if let core::task::Poll::Ready(Ok(())) = snap_fut.as_mut().poll(cx) {
            return core::task::Poll::Ready(Some(SupervisorEvent::PrimaryChanged));
        }
        if tick_fut.as_mut().poll(cx).is_ready() {
            return core::task::Poll::Ready(Some(SupervisorEvent::ReconcileTick));
        }
        core::task::Poll::Pending
    })
    .await
}
