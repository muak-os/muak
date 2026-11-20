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
///
/// Returns `None` once the supervisor command channel is closed.
pub(super) async fn wait(
    cmd_rx: &mut mpsc::Receiver<SupervisorCommand>,
    event_rx: &mut mpsc::Receiver<Event>,
    primary_snap_rx: &mut Option<watch::Receiver<Arc<Snapshot>>>,
    reconcile: &mut Interval,
) -> Option<SupervisorEvent> {
    let mut cmd_fut = core::pin::pin!(cmd_rx.recv());
    // A closed monitor channel must not stop the supervisor: the stream is renewable by
    // the supervisor's monitor subscription, so the pending arm keeps this future from ever
    // completing.
    let mut event_fut = core::pin::pin!(async {
        match event_rx.recv().await {
            Some(event) => event,
            None => core::future::pending().await,
        }
    });
    let mut snap_fut = core::pin::pin!(async {
        match primary_snap_rx.as_mut() {
            Some(rx) => rx.changed().await,
            None => core::future::pending().await,
        }
    });
    let mut tick_fut = core::pin::pin!(reconcile.tick());

    core::future::poll_fn(|cx| {
        if let core::task::Poll::Ready(command) = cmd_fut.as_mut().poll(cx) {
            return core::task::Poll::Ready(command.map(SupervisorEvent::Command));
        }
        if let core::task::Poll::Ready(event) = event_fut.as_mut().poll(cx) {
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

#[cfg(test)]
mod tests {
    use core::time::Duration;

    use tokio::time::{interval, timeout};

    use super::*;

    #[tokio::test]
    async fn wait_returns_none_when_commands_close() {
        // ARRANGE
        let (cmd_tx, mut cmd_rx) = mpsc::channel::<SupervisorCommand>(1);
        let (_event_tx, mut event_rx) = mpsc::channel::<Event>(1);
        let mut snap = None;
        let mut reconcile = interval(Duration::from_secs(30));
        reconcile.tick().await;
        drop(cmd_tx);

        // ACT
        let result = timeout(
            Duration::from_secs(1),
            wait(&mut cmd_rx, &mut event_rx, &mut snap, &mut reconcile),
        )
        .await;

        // ASSERT
        assert!(matches!(result, Ok(None)));
    }

    #[tokio::test]
    async fn wait_keeps_running_when_the_monitor_channel_closes() {
        // ARRANGE
        let (_cmd_tx, mut cmd_rx) = mpsc::channel::<SupervisorCommand>(1);
        let (event_tx, mut event_rx) = mpsc::channel::<Event>(1);
        let mut snap = None;
        let mut reconcile = interval(Duration::from_millis(10));
        drop(event_tx);

        // ACT
        let result = timeout(
            Duration::from_secs(1),
            wait(&mut cmd_rx, &mut event_rx, &mut snap, &mut reconcile),
        )
        .await;

        // ASSERT
        assert!(matches!(result, Ok(Some(SupervisorEvent::ReconcileTick))));
    }
}
