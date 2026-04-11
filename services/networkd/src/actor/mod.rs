//! Asynchronous actor that owns and drives all network state.

mod bridge;
mod commands;
mod dhcp;
mod discovery;
mod events;
mod provision;
mod slaac;
mod startup;
mod state;
mod static_ip;

use anyhow::Result;
pub use commands::NetworkCommand;
use state::NetworkActor;
pub use state::NetworkSnapshot;
use tokio::sync::{mpsc, watch};

use crate::monitor::{self, NetworkEvent};

#[derive(Clone)]
pub struct NetworkActorHandle {
    tx: mpsc::Sender<NetworkCommand>,
}

impl NetworkActorHandle {
    async fn initialize(&self) -> Result<()> {
        let (reply, rx) = tokio::sync::oneshot::channel();
        self.tx.send(NetworkCommand::Initialize { reply }).await?;
        rx.await??;
        Ok(())
    }

    pub async fn initialize_with_retry(&self) -> Result<()> {
        let base_delay = std::time::Duration::from_secs(1);
        let max_delay = std::time::Duration::from_secs(10);

        for attempt in 1u32.. {
            match self.try_initialize(attempt, base_delay, max_delay).await {
                Some(ok) => return ok,
                None => continue,
            }
        }

        unreachable!()
    }

    async fn try_initialize(
        &self,
        attempt: u32,
        base_delay: std::time::Duration,
        max_delay: std::time::Duration,
    ) -> Option<Result<()>> {
        if self.initialize().await.is_ok() {
            println!("Network initialized successfully on attempt {}", attempt);
            return Some(Ok(()));
        }
        eprintln!("Network initialization failed (attempt {})", attempt);

        let delay = retry_delay(base_delay, max_delay, attempt);
        println!("Retrying in {:?}...", delay);
        tokio::time::sleep(delay).await;
        None
    }
}

fn retry_delay(
    base: std::time::Duration,
    max: std::time::Duration,
    attempt: u32,
) -> std::time::Duration {
    let multiplier = 1u32 << attempt.saturating_sub(1).min(5);
    base.checked_mul(multiplier).unwrap_or(max).min(max)
}

pub async fn start_network_actor() -> Result<NetworkActorHandle> {
    let (connection, handle, _) = rtnetlink::new_connection()?;
    tokio::spawn(connection);

    let (cmd_tx, cmd_rx) = mpsc::channel(32);
    let (watch_tx, _) = watch::channel(NetworkSnapshot::empty());

    let event_rx = start_events_monitor(handle.clone()).await;

    handle_network_actions(handle, cmd_rx, event_rx, cmd_tx.clone(), watch_tx);

    Ok(NetworkActorHandle { tx: cmd_tx })
}

async fn start_events_monitor(handle: rtnetlink::Handle) -> Option<mpsc::Receiver<NetworkEvent>> {
    let config = monitor::MonitorConfig::default();
    match monitor::start_monitor(handle, config).await {
        Ok(rx) => {
            println!("Network event monitoring enabled");
            Some(rx)
        }
        Err(e) => {
            eprintln!("Failed to start network monitor: {}", e);
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
                    println!("Network actor shutting down");
                    break;
                }
            }
        }
    });
}
