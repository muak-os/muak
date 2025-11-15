use crate::log;
use anyhow::Result;
use rtnetlink::{Handle, new_connection};
use std::collections::HashMap;
use tokio::sync::{mpsc, oneshot, watch};

use super::bridge::attach_to_bridge;
use super::config::LAN_BRIDGE_NAME;
use super::dhcp::run_dhcp_client;
use super::dns::configure_dns;
use super::interface::{LinkState as OldLinkState, discover_ethernet_interfaces};
use super::model::*;
use super::ops::*;
use super::tap::{bring_up_tap, create_tap, delete_tap};

pub enum NetworkCommand {
    Initialize {
        reply: oneshot::Sender<Result<()>>,
    },
    SetupBridge {
        reply: oneshot::Sender<Result<()>>,
    },
    AddTap {
        name: String,
        reply: oneshot::Sender<Result<InterfaceSnapshot>>,
    },
    DeleteTap {
        name: String,
        reply: oneshot::Sender<Result<()>>,
    },
    AcquireDhcp {
        iface: String,
        reply: oneshot::Sender<Result<InterfaceSnapshot>>,
    },
    RenewLease {
        iface: String,
    },
    Snapshot {
        reply: oneshot::Sender<NetworkSnapshot>,
    },
}

#[derive(Clone)]
pub struct NetworkActorHandle {
    tx: mpsc::Sender<NetworkCommand>,
    watch_rx: watch::Receiver<NetworkSnapshot>,
}

impl NetworkActorHandle {
    pub async fn initialize(&self) -> Result<()> {
        let (reply, rx) = oneshot::channel();
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
                    log!(
                        "network",
                        "Network initialized successfully on attempt {}",
                        attempt
                    );
                    return Ok(());
                }
                Err(e) => {
                    log!(
                        "network",
                        "Network initialization failed (attempt {}): {}",
                        attempt,
                        e
                    );

                    let delay = std::cmp::min(
                        base_delay
                            .checked_mul(1u32 << attempt.saturating_sub(1).min(5))
                            .unwrap_or(max_delay),
                        max_delay,
                    );

                    log!("network", "Retrying in {:?}...", delay);
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

    pub async fn add_tap(&self, name: &str) -> Result<InterfaceSnapshot> {
        let (reply, rx) = oneshot::channel();
        self.tx
            .send(NetworkCommand::AddTap {
                name: name.to_string(),
                reply,
            })
            .await?;
        rx.await?
    }

    pub async fn delete_tap(&self, name: &str) -> Result<()> {
        let (reply, rx) = oneshot::channel();
        self.tx
            .send(NetworkCommand::DeleteTap {
                name: name.to_string(),
                reply,
            })
            .await?;
        rx.await??;
        Ok(())
    }

    pub async fn acquire_dhcp(&self, iface: &str) -> Result<InterfaceSnapshot> {
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
        let (reply, rx) = oneshot::channel();
        let _ = self.tx.send(NetworkCommand::Snapshot { reply }).await;
        rx.await.unwrap_or_else(|_| self.watch_rx.borrow().clone())
    }

    pub fn subscribe(&self) -> watch::Receiver<NetworkSnapshot> {
        self.watch_rx.clone()
    }
}

pub async fn start_network_actor() -> Result<NetworkActorHandle> {
    let (connection, handle, _) = new_connection()?;
    tokio::spawn(connection);
    let (tx, mut rx) = mpsc::channel(32);
    let (watch_tx, watch_rx) = watch::channel(NetworkSnapshot::empty());

    let tx_clone = tx.clone();
    tokio::spawn(async move {
        let mut state = NetworkSnapshot::empty();
        let mut iface_map: HashMap<String, InterfaceSnapshot> = HashMap::new();
        while let Some(cmd) = rx.recv().await {
            match cmd {
                NetworkCommand::Initialize { reply } => {
                    // Initialize network (discover interfaces + DHCP + bridge)
                    let res = initialize_impl(&handle, &mut state, &mut iface_map, &watch_tx).await;

                    let final_result = if res.is_ok() {
                        // Try to acquire DHCP
                        let dhcp_result = if let Some(primary) = state.primary.clone() {
                            acquire_dhcp_impl(
                                &handle,
                                &mut state,
                                &mut iface_map,
                                &primary,
                                &watch_tx,
                                &tx_clone,
                            )
                            .await
                            .map(|_| ())
                        } else {
                            Ok(())
                        };

                        // If DHCP succeeded, setup bridge
                        if dhcp_result.is_ok() {
                            match setup_bridge_impl(&handle, &mut state).await {
                                Ok(()) => {
                                    state.state = NetworkStateKind::Ready;
                                    let _ = watch_tx.send(state.clone());
                                    log!(
                                        "network",
                                        "Full network initialization complete (BridgeReady)"
                                    );
                                    Ok(())
                                }
                                Err(e) => {
                                    log!("network", "Bridge setup failed: {}", e);
                                    state.state = NetworkStateKind::Degraded;
                                    let _ = watch_tx.send(state.clone());
                                    Err(e)
                                }
                            }
                        } else {
                            dhcp_result
                        }
                    } else {
                        res
                    };

                    let _ = reply.send(final_result);
                }
                NetworkCommand::SetupBridge { reply } => {
                    let res = setup_bridge_impl(&handle, &mut state).await;
                    let _ = reply.send(res);
                }
                NetworkCommand::AddTap { name, reply } => {
                    let res =
                        add_tap_impl(&handle, &mut state, &mut iface_map, &name, &watch_tx).await;
                    let _ = reply.send(res);
                }
                NetworkCommand::DeleteTap { name, reply } => {
                    let res =
                        delete_tap_impl(&handle, &mut state, &mut iface_map, &name, &watch_tx)
                            .await;
                    let _ = reply.send(res);
                }
                NetworkCommand::AcquireDhcp { iface, reply } => {
                    let res = acquire_dhcp_impl(
                        &handle,
                        &mut state,
                        &mut iface_map,
                        &iface,
                        &watch_tx,
                        &tx_clone,
                    )
                    .await;
                    let _ = reply.send(res);
                }
                NetworkCommand::RenewLease { iface } => {
                    let _ =
                        renew_lease_impl(&handle, &mut state, &mut iface_map, &iface, &watch_tx)
                            .await;
                }
                NetworkCommand::Snapshot { reply } => {
                    let _ = reply.send(state.clone());
                }
            }
        }
    });

    Ok(NetworkActorHandle { tx, watch_rx })
}

async fn initialize_impl(
    handle: &Handle,
    snap: &mut NetworkSnapshot,
    iface_map: &mut HashMap<String, InterfaceSnapshot>,
    watch_tx: &watch::Sender<NetworkSnapshot>,
) -> Result<()> {
    log!("network", "Actor: initialize start");
    snap.state = NetworkStateKind::Initializing;
    let _ = watch_tx.send(snap.clone());
    let discovered = discover_ethernet_interfaces(handle).await?;
    if discovered.is_empty() {
        snap.state = NetworkStateKind::Degraded;
        let _ = watch_tx.send(snap.clone());
        anyhow::bail!("no ethernet interfaces")
    }
    // Simple selection: first Up else first
    let primary = discovered
        .iter()
        .find(|i| i.link_state == OldLinkState::Up)
        .unwrap_or(&discovered[0]);
    snap.primary = Some(primary.name.clone());
    snap.backups = discovered
        .iter()
        .filter(|i| i.name != primary.name)
        .map(|i| i.name.clone())
        .collect();
    for iface in discovered {
        iface_map.insert(
            iface.name.clone(),
            InterfaceSnapshot {
                name: iface.name.clone(),
                index: iface.index,
                mac: iface.mac_address,
                link: match iface.link_state {
                    OldLinkState::Up => LinkStateKind::Up,
                    OldLinkState::Down => LinkStateKind::Down,
                },
                ip: None,
                lease: None,
            },
        );
    }
    snap.interfaces = iface_map.values().cloned().collect();
    snap.state = NetworkStateKind::Operational;
    let _ = watch_tx.send(snap.clone());
    log!(
        "network",
        "Actor: initialize complete primary={:?}",
        snap.primary
    );

    Ok(())
}

async fn setup_bridge_impl(handle: &Handle, snap: &mut NetworkSnapshot) -> Result<()> {
    let primary = snap
        .primary
        .clone()
        .ok_or_else(|| anyhow::anyhow!("no primary interface"))?;

    super::bridge::ensure_bridge_with_ip_transfer(handle, LAN_BRIDGE_NAME, &primary).await?;
    log!(
        "network",
        "Actor: bridge ensure complete br={} primary={}",
        LAN_BRIDGE_NAME,
        primary
    );
    Ok(())
}

async fn add_tap_impl(
    handle: &Handle,
    snap: &mut NetworkSnapshot,
    iface_map: &mut HashMap<String, InterfaceSnapshot>,
    name: &str,
    watch_tx: &watch::Sender<NetworkSnapshot>,
) -> Result<InterfaceSnapshot> {
    create_tap(name).await?;
    bring_up_tap(handle, name).await?;
    attach_to_bridge(handle, name, LAN_BRIDGE_NAME).await?;
    let index = ensure_link_up(handle, name).await?;
    let snapshot = InterfaceSnapshot {
        name: name.to_string(),
        index,
        mac: [0, 0, 0, 0, 0, 0],
        link: LinkStateKind::Up,
        ip: None,
        lease: None,
    };
    iface_map.insert(name.to_string(), snapshot.clone());
    snap.interfaces = iface_map.values().cloned().collect();
    let _ = watch_tx.send(snap.clone());
    Ok(snapshot)
}

async fn delete_tap_impl(
    handle: &Handle,
    snap: &mut NetworkSnapshot,
    iface_map: &mut HashMap<String, InterfaceSnapshot>,
    name: &str,
    watch_tx: &watch::Sender<NetworkSnapshot>,
) -> Result<()> {
    delete_tap(handle, name).await?;
    iface_map.remove(name);
    snap.interfaces = iface_map.values().cloned().collect();
    let _ = watch_tx.send(snap.clone());
    Ok(())
}

fn schedule_lease_timers(tx: mpsc::Sender<NetworkCommand>, iface: String, lease: DhcpLease) {
    let renew_deadline = lease.obtained_at + lease.renewal_time;
    let rebind_deadline = lease.obtained_at + lease.rebind_time;
    let expiry_deadline = lease.expiry();

    // Renewal task
    tokio::spawn({
        let iface = iface.clone();
        let tx = tx.clone();
        async move {
            let now = std::time::SystemTime::now();
            if let Ok(dur) = renew_deadline.duration_since(now) {
                tokio::time::sleep(dur).await;
            } else {
                return;
            }
            log!("network", "Lease renewal attempt for {}", iface);
            // Send renewal command to actor
            let _ = tx.send(NetworkCommand::RenewLease { iface }).await;
        }
    });

    // Rebind task
    tokio::spawn({
        let iface = iface.clone();
        let tx = tx.clone();
        async move {
            let now = std::time::SystemTime::now();
            if let Ok(dur) = rebind_deadline.duration_since(now) {
                tokio::time::sleep(dur).await;
            } else {
                return;
            }
            log!("network", "Lease rebind attempt for {}", iface);
            // Send renewal command to actor
            let _ = tx.send(NetworkCommand::RenewLease { iface }).await;
        }
    });

    // Expiry task (fallback)
    tokio::spawn({
        let iface = iface.clone();
        let tx = tx.clone();
        async move {
            let now = std::time::SystemTime::now();
            if let Ok(dur) = expiry_deadline.duration_since(now) {
                tokio::time::sleep(dur).await;
            } else {
                return;
            }
            log!("network", "Lease expired for {} - reacquiring", iface);
            // Send renewal command to actor
            let _ = tx.send(NetworkCommand::RenewLease { iface }).await;
        }
    });
}

async fn renew_lease_impl(
    handle: &Handle,
    snap: &mut NetworkSnapshot,
    iface_map: &mut HashMap<String, InterfaceSnapshot>,
    iface: &str,
    watch_tx: &watch::Sender<NetworkSnapshot>,
) -> Result<()> {
    log!("network", "Renewing DHCP lease for {}", iface);

    let mac = iface_map
        .get(iface)
        .map(|i| i.mac)
        .ok_or_else(|| anyhow::anyhow!("interface not tracked"))?;

    // Attempt to renew the lease
    match run_dhcp_client(iface, &mac).await {
        Ok((ip_cfg, lease)) => {
            let index = iface_map.get(iface).unwrap().index;

            // Apply the new configuration
            ensure_addr(handle, index, ip_cfg.address, ip_cfg.prefix_len).await?;
            if let Some(gw) = ip_cfg.gateway {
                ensure_default_route_v4(handle, gw).await?;
            }
            if !ip_cfg.dns.is_empty() {
                configure_dns(&ip_cfg.dns)?;
            }

            // Update state
            if let Some(existing) = iface_map.get_mut(iface) {
                existing.ip = Some(ip_cfg.clone());
                existing.lease = Some(lease.clone());
            }
            snap.interfaces = iface_map.values().cloned().collect();
            let _ = watch_tx.send(snap.clone());

            log!("network", "DHCP lease renewed for {}", iface);
            Ok(())
        }
        Err(e) => {
            log!("network", "DHCP renewal failed for {}: {}", iface, e);
            Err(anyhow::anyhow!("DHCP renewal failed: {}", e))
        }
    }
}

async fn acquire_dhcp_impl(
    handle: &Handle,
    snap: &mut NetworkSnapshot,
    iface_map: &mut HashMap<String, InterfaceSnapshot>,
    iface: &str,
    watch_tx: &watch::Sender<NetworkSnapshot>,
    tx: &mpsc::Sender<NetworkCommand>,
) -> Result<InterfaceSnapshot> {
    let index = ensure_link_up(handle, iface).await?;
    let mac = iface_map
        .get(iface)
        .map(|i| i.mac)
        .ok_or_else(|| anyhow::anyhow!("interface not tracked"))?;
    let (ip_cfg, lease) = run_dhcp_client(iface, &mac).await?;

    ensure_addr(handle, index, ip_cfg.address, ip_cfg.prefix_len).await?;
    if let Some(gw) = ip_cfg.gateway {
        ensure_default_route_v4(handle, gw).await?;
    }
    if !ip_cfg.dns.is_empty() {
        configure_dns(&ip_cfg.dns)?;
    }

    if let Some(existing) = iface_map.get_mut(iface) {
        existing.ip = Some(ip_cfg.clone());
        existing.lease = Some(lease.clone());
    }
    snap.interfaces = iface_map.values().cloned().collect();
    let _ = watch_tx.send(snap.clone());

    schedule_lease_timers(tx.clone(), iface.to_string(), lease.clone());

    Ok(iface_map.get(iface).unwrap().clone())
}
