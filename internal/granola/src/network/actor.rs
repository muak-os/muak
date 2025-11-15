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

    tokio::spawn(async move {
        let mut state = NetworkSnapshot::empty();
        let mut iface_map: HashMap<String, InterfaceSnapshot> = HashMap::new();
        while let Some(cmd) = rx.recv().await {
            match cmd {
                NetworkCommand::Initialize { reply } => {
                    let res = initialize_impl(&handle, &mut state, &mut iface_map, &watch_tx).await;
                    let _ = reply.send(res);
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
                    let res =
                        acquire_dhcp_impl(&handle, &mut state, &mut iface_map, &iface, &watch_tx)
                            .await;
                    let _ = reply.send(res);
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
    let discovered = discover_ethernet_interfaces(handle)
        .await
        .map_err(|e| anyhow::anyhow!(e.to_string()))?;
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
    snap.state = NetworkStateKind::Ready;
    let _ = watch_tx.send(snap.clone());
    log!(
        "network",
        "Actor: initialize complete primary={:?}",
        snap.primary
    );

    // Auto DHCP on primary
    if let Some(primary) = snap.primary.clone() {
        if let Err(e) = acquire_dhcp_impl(handle, snap, iface_map, &primary, watch_tx).await {
            log!("network", "DHCP acquisition failed: {}", e);
        }
    }

    Ok(())
}

async fn setup_bridge_impl(handle: &Handle, snap: &mut NetworkSnapshot) -> Result<()> {
    let primary = snap
        .primary
        .clone()
        .ok_or_else(|| anyhow::anyhow!("no primary interface"))?;
    // Use new bridge IP transfer helper
    super::bridge::ensure_bridge_with_ip_transfer(handle, LAN_BRIDGE_NAME, &primary)
        .await
        .map_err(|e| anyhow::anyhow!(e.to_string()))?;
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
    create_tap(name)
        .await
        .map_err(|e| anyhow::anyhow!(e.to_string()))?;
    bring_up_tap(handle, name)
        .await
        .map_err(|e| anyhow::anyhow!(e.to_string()))?;
    attach_to_bridge(handle, name, LAN_BRIDGE_NAME)
        .await
        .map_err(|e| anyhow::anyhow!(e.to_string()))?;
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
    delete_tap(handle, name)
        .await
        .map_err(|e| anyhow::anyhow!(e.to_string()))?;
    iface_map.remove(name);
    snap.interfaces = iface_map.values().cloned().collect();
    let _ = watch_tx.send(snap.clone());
    Ok(())
}

fn schedule_lease_timers(
    handle: Handle,
    iface: String,
    tx: watch::Sender<NetworkSnapshot>,
    lease: DhcpLease,
) {
    let renew_deadline = lease.obtained_at + lease.renewal_time;
    let rebind_deadline = lease.obtained_at + lease.rebind_time;
    let expiry_deadline = lease.expiry();

    // Renewal task
    tokio::spawn({
        let iface = iface.clone();
        let tx = tx.clone();
        let handle = handle.clone();
        async move {
            let now = std::time::SystemTime::now();
            if let Ok(dur) = renew_deadline.duration_since(now) {
                tokio::time::sleep(dur).await;
            } else {
                return;
            }
            log!("network", "Lease renewal attempt for {}", iface);
            // Re-run DHCP request (unicast ideal; for simplicity full discover again)
            // Fetch current snapshot to get MAC
            let snap = tx.borrow().clone();
            let mac = snap
                .interfaces
                .iter()
                .find(|i| i.name == iface)
                .map(|i| i.mac);
            if let Some(mac) = mac {
                if let Ok((ip_cfg, new_lease)) = run_dhcp_client(&iface, &mac)
                    .await
                    .map_err(|e| anyhow::anyhow!(e.to_string()))
                {
                    apply_lease(&handle, &tx, &iface, ip_cfg, new_lease);
                }
            }
        }
    });

    // Rebind task
    tokio::spawn({
        let iface = iface.clone();
        let tx = tx.clone();
        let handle = handle.clone();
        async move {
            let now = std::time::SystemTime::now();
            if let Ok(dur) = rebind_deadline.duration_since(now) {
                tokio::time::sleep(dur).await;
            } else {
                return;
            }
            log!("network", "Lease rebind attempt for {}", iface);
            let snap = tx.borrow().clone();
            let mac = snap
                .interfaces
                .iter()
                .find(|i| i.name == iface)
                .map(|i| i.mac);
            if let Some(mac) = mac {
                if let Ok((ip_cfg, new_lease)) = run_dhcp_client(&iface, &mac)
                    .await
                    .map_err(|e| anyhow::anyhow!(e.to_string()))
                {
                    apply_lease(&handle, &tx, &iface, ip_cfg, new_lease);
                }
            }
        }
    });

    // Expiry task (fallback)
    tokio::spawn({
        let iface = iface.clone();
        let tx = tx.clone();
        let handle = handle.clone();
        async move {
            let now = std::time::SystemTime::now();
            if let Ok(dur) = expiry_deadline.duration_since(now) {
                tokio::time::sleep(dur).await;
            } else {
                return;
            }
            log!("network", "Lease expired for {} - reacquiring", iface);
            let snap = tx.borrow().clone();
            let mac = snap
                .interfaces
                .iter()
                .find(|i| i.name == iface)
                .map(|i| i.mac);
            if let Some(mac) = mac {
                if let Ok((ip_cfg, new_lease)) = run_dhcp_client(&iface, &mac)
                    .await
                    .map_err(|e| anyhow::anyhow!(e.to_string()))
                {
                    apply_lease(&handle, &tx, &iface, ip_cfg, new_lease);
                }
            }
        }
    });
}

fn apply_lease(
    handle: &Handle,
    tx: &watch::Sender<NetworkSnapshot>,
    iface: &str,
    ip_cfg: IpConfig,
    lease: DhcpLease,
) {
    let snap = tx.borrow().clone();
    if let Some(idx) = snap.interfaces.iter().position(|i| i.name == iface) {
        let index = snap.interfaces[idx].index;
        tokio::spawn({
            let handle = handle.clone();
            let tx = tx.clone();
            let mut snap_local = snap.clone();
            let iface_name = iface.to_string();
            async move {
                if ensure_addr(&handle, index, ip_cfg.address, ip_cfg.prefix_len)
                    .await
                    .is_ok()
                {
                    if let Some(gw) = ip_cfg.gateway {
                        let _ = ensure_default_route_v4(&handle, gw).await;
                    }
                    if !ip_cfg.dns.is_empty() {
                        let _ = configure_dns(&ip_cfg.dns);
                    }
                    snap_local.interfaces[idx].ip = Some(ip_cfg.clone());
                    snap_local.interfaces[idx].lease = Some(lease.clone());
                    let _ = tx.send(snap_local.clone());
                    schedule_lease_timers(handle.clone(), iface_name, tx.clone(), lease.clone());
                }
            }
        });
    }
}

async fn acquire_dhcp_impl(
    handle: &Handle,
    snap: &mut NetworkSnapshot,
    iface_map: &mut HashMap<String, InterfaceSnapshot>,
    iface: &str,
    watch_tx: &watch::Sender<NetworkSnapshot>,
) -> Result<InterfaceSnapshot> {
    let index = ensure_link_up(handle, iface).await?;
    let mac = iface_map
        .get(iface)
        .map(|i| i.mac)
        .ok_or_else(|| anyhow::anyhow!("interface not tracked"))?;
    let (ip_cfg, lease) = run_dhcp_client(iface, &mac)
        .await
        .map_err(|e| anyhow::anyhow!(e.to_string()))?;

    ensure_addr(handle, index, ip_cfg.address, ip_cfg.prefix_len).await?;
    if let Some(gw) = ip_cfg.gateway {
        ensure_default_route_v4(handle, gw).await?;
    }
    if !ip_cfg.dns.is_empty() {
        configure_dns(&ip_cfg.dns).map_err(|e| anyhow::anyhow!(e.to_string()))?;
    }

    if let Some(existing) = iface_map.get_mut(iface) {
        existing.ip = Some(ip_cfg.clone());
        existing.lease = Some(lease.clone());
    }
    snap.interfaces = iface_map.values().cloned().collect();
    let _ = watch_tx.send(snap.clone());

    // Schedule real renewal + rebind timers
    schedule_lease_timers(
        handle.clone(),
        iface.to_string(),
        watch_tx.clone(),
        lease.clone(),
    );

    Ok(iface_map.get(iface).unwrap().clone())
}
