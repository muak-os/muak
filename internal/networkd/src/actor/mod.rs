mod commands;
mod events;
mod operations;
mod state;

use anyhow::Result;
use rtnetlink::new_connection;
use tokio::sync::{mpsc, oneshot, watch};

use crate::model::{ConnectivityResult, InterfaceSnapshot, NetworkSnapshot};
use crate::monitor::{self, NetworkEvent};

pub use commands::NetworkCommand;
use state::NetworkActor;

#[derive(Clone)]
pub struct NetworkActorHandle {
    tx: mpsc::Sender<NetworkCommand>,
    watch_rx: watch::Receiver<NetworkSnapshot>,
}

#[allow(dead_code)]
impl NetworkActorHandle {
    async fn initialize(&self) -> Result<()> {
        let (reply, rx) = tokio::sync::oneshot::channel();
        self.tx.send(NetworkCommand::Initialize { reply }).await?;
        rx.await??;
        Ok(())
    }

    pub async fn initialize_with_retry(&self) -> Result<()> {
        let mut attempt = 0u32;
        let base_delay = std::time::Duration::from_secs(1);
        let max_delay = std::time::Duration::from_secs(10);

        loop {
            attempt += 1;

            match self.initialize().await {
                Ok(()) => {
                    kmsg::info!(
                        @ "network",
                        "Network initialized successfully on attempt {}",
                        attempt
                    );
                    return Ok(());
                }
                Err(e) => {
                    kmsg::warn!(
                        @ "network",
                        "Network initialization failed (attempt {}): {}",
                        attempt,
                        e
                    );

                    let multiplier = 1u32 << attempt.saturating_sub(1).min(5);
                    let delay = base_delay
                        .checked_mul(multiplier)
                        .unwrap_or(max_delay)
                        .min(max_delay);

                    kmsg::info!(@ "network", "Retrying in {:?}...", delay);
                    tokio::time::sleep(delay).await;
                }
            }
        }
    }

    pub async fn setup_bridge(&self) -> Result<()> {
        let (reply, rx) = oneshot::channel();
        self.tx.send(NetworkCommand::SetupBridge { reply }).await?;
        rx.await??;
        Ok(())
    }

    pub async fn add_tap(&self, name: String) -> Result<InterfaceSnapshot> {
        let (reply, rx) = oneshot::channel();
        self.tx.send(NetworkCommand::AddTap { name, reply }).await?;
        rx.await?
    }

    pub async fn delete_tap(&self, name: String) -> Result<()> {
        let (reply, rx) = oneshot::channel();
        self.tx
            .send(NetworkCommand::DeleteTap { name, reply })
            .await?;
        rx.await??;
        Ok(())
    }

    pub async fn acquire_dhcp(&self, iface: &str) -> Result<crate::model::InterfaceSnapshot> {
        let (reply, rx) = oneshot::channel();
        self.tx
            .send(NetworkCommand::AcquireDhcp {
                iface: iface.to_string(),
                reply,
            })
            .await?;
        rx.await?
    }

    pub async fn snapshot(&self) -> NetworkSnapshot {
        let (reply, rx) = tokio::sync::oneshot::channel();
        let _ = self.tx.send(NetworkCommand::Snapshot { reply }).await;
        rx.await.unwrap_or_else(|_| self.watch_rx.borrow().clone())
    }

    pub fn subscribe(&self) -> watch::Receiver<NetworkSnapshot> {
        self.watch_rx.clone()
    }

    pub async fn check_connectivity(&self) -> ConnectivityResult {
        let (reply, rx) = oneshot::channel();
        let _ = self
            .tx
            .send(NetworkCommand::CheckConnectivity { reply })
            .await;
        rx.await.unwrap_or_default()
    }
}

pub async fn start_network_actor() -> Result<NetworkActorHandle> {
    // Create netlink connection
    let (connection, handle, _) = new_connection()?;
    tokio::spawn(connection);

    let (cmd_tx, cmd_rx) = mpsc::channel(32);
    let (watch_tx, watch_rx) = watch::channel(NetworkSnapshot::empty());

    let event_rx = start_events_monitor(handle.clone()).await;

    handle_network_actions(handle, cmd_rx, event_rx, cmd_tx.clone(), watch_tx);

    Ok(NetworkActorHandle {
        tx: cmd_tx,
        watch_rx,
    })
}

async fn start_events_monitor(handle: rtnetlink::Handle) -> Option<mpsc::Receiver<NetworkEvent>> {
    let config = monitor::MonitorConfig::default();
    match monitor::start_monitor(handle, config).await {
        Ok(rx) => {
            kmsg::info!(@ "network", "Network event monitoring enabled");
            Some(rx)
        }
        Err(e) => {
            kmsg::warn!(@ "network", "Failed to start network monitor: {}", e);
            None
        }
    }
}

fn handle_network_actions(
    handle: rtnetlink::Handle,
    mut cmd_rx: mpsc::Receiver<NetworkCommand>,
    mut event_rx: Option<mpsc::Receiver<NetworkEvent>>,
    cmd_tx: mpsc::Sender<NetworkCommand>,
    watch_tx: watch::Sender<NetworkSnapshot>,
) {
    tokio::spawn(async move {
        let mut actor = NetworkActor::new(handle, watch_tx);

        loop {
            tokio::select! {
                Some(cmd) = cmd_rx.recv() => {
                    actor.handle_command(cmd, &cmd_tx).await;
                }

                Some(event) = async {
                    match &mut event_rx {
                        Some(rx) => rx.recv().await,
                        None => std::future::pending().await,
                    }
                } => {
                    actor.handle_event(event).await;
                }

                else => {
                    kmsg::info!(@ "network", "Network actor shutting down");
                    break;
                }
            }
        }
    });
}
