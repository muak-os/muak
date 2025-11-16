use crate::log;
use anyhow::Result;
use futures::stream::StreamExt;
use netlink_packet_core::NetlinkPayload;
use netlink_packet_route::{
    RouteNetlinkMessage,
    link::{LinkAttribute, LinkFlag, LinkMessage},
};
use rtnetlink::Handle;
use std::collections::HashMap;
use tokio::sync::mpsc;

#[derive(Debug, Clone)]
pub enum NetworkEvent {
    LinkUp {
        name: String,
        index: u32,
    },
    LinkDown {
        name: String,
        index: u32,
    },
    LinkAdded {
        name: String,
        index: u32,
        mac: [u8; 6],
    },
    LinkDeleted {
        name: String,
        index: u32,
    },
}

/// Monitor configuration
pub struct MonitorConfig {
    /// Monitor link state changes (up/down)
    pub monitor_link_state: bool,
    /// Monitor link additions/removals
    pub monitor_link_changes: bool,
}

impl Default for MonitorConfig {
    fn default() -> Self {
        Self {
            monitor_link_state: true,
            monitor_link_changes: true,
        }
    }
}

/// Starts the network event monitor
///
/// This function spawns a background task that listens to netlink events
/// and sends relevant network events through the returned channel.
///
/// # Returns
/// A receiver channel that will receive NetworkEvent notifications
pub async fn start_monitor(
    _handle: Handle,
    config: MonitorConfig,
) -> Result<mpsc::Receiver<NetworkEvent>> {
    let (tx, rx) = mpsc::channel(32);

    // Create a new connection specifically for monitoring
    let (connection, handle, mut messages) = rtnetlink::new_connection()?;

    // Spawn the connection task
    tokio::spawn(async move {
        let _ = connection.await;
    });

    // Track interface state to detect changes
    let mut link_states: HashMap<u32, (String, bool)> = HashMap::new();

    // Spawn event processor
    tokio::spawn(async move {
        // Initial scan to populate link states
        if let Err(e) = initial_scan(&handle, &mut link_states).await {
            log!("network", "Initial interface scan failed: {}", e);
        }

        // Process incoming netlink messages
        while let Some((message, _)) = messages.next().await {
            if let NetlinkPayload::InnerMessage(route_msg) = message.payload {
                if let Err(e) = handle_message(route_msg, &tx, &config, &mut link_states).await {
                    log!("network", "Error handling netlink message: {}", e);
                }
            }
        }
    });

    log!("network", "Network event monitor started");
    Ok(rx)
}

/// Perform initial scan to populate link state tracking
async fn initial_scan(
    handle: &Handle,
    link_states: &mut HashMap<u32, (String, bool)>,
) -> Result<()> {
    use futures::stream::TryStreamExt;

    let mut links = handle.link().get().execute();
    while let Some(link) = links.try_next().await? {
        if let Some((name, index, _)) = extract_link_info(&link) {
            if super::interface::is_ethernet_interface(&name) {
                let is_up = is_link_up(&link);
                link_states.insert(index, (name.clone(), is_up));
                log!(
                    "network",
                    "Initial state: {} (index {}) = {}",
                    name,
                    index,
                    if is_up { "up" } else { "down" }
                );
            }
        }
    }
    Ok(())
}

/// Handle individual netlink messages
async fn handle_message(
    msg: RouteNetlinkMessage,
    tx: &mpsc::Sender<NetworkEvent>,
    config: &MonitorConfig,
    link_states: &mut HashMap<u32, (String, bool)>,
) -> Result<()> {
    match msg {
        RouteNetlinkMessage::NewLink(link_msg) => {
            if config.monitor_link_changes || config.monitor_link_state {
                handle_new_link(link_msg, tx, link_states).await?;
            }
        }
        RouteNetlinkMessage::DelLink(link_msg) => {
            if config.monitor_link_changes {
                handle_del_link(link_msg, tx, link_states).await?;
            }
        }
        _ => {
            // Ignore other message types (routes, neighbors, etc.)
        }
    }
    Ok(())
}

/// Extract interface name from link message
fn extract_link_info(msg: &LinkMessage) -> Option<(String, u32, Option<[u8; 6]>)> {
    let index = msg.header.index;
    let mut name = None;
    let mut mac = None;

    for attr in &msg.attributes {
        match attr {
            LinkAttribute::IfName(n) => name = Some(n.clone()),
            LinkAttribute::Address(addr) if addr.len() == 6 => {
                let mut mac_arr = [0u8; 6];
                mac_arr.copy_from_slice(&addr[..6]);
                mac = Some(mac_arr);
            }
            _ => {}
        }
    }

    name.map(|n| (n, index, mac))
}

fn is_link_up(msg: &LinkMessage) -> bool {
    msg.header
        .flags
        .iter()
        .any(|flag| matches!(flag, LinkFlag::Up))
}

/// Handle NewLink messages (link state changes or new links)
async fn handle_new_link(
    msg: LinkMessage,
    tx: &mpsc::Sender<NetworkEvent>,
    link_states: &mut HashMap<u32, (String, bool)>,
) -> Result<()> {
    if let Some((name, index, mac)) = extract_link_info(&msg) {
        // Filter out non-ethernet interfaces
        if !super::interface::is_ethernet_interface(&name) {
            return Ok(());
        }

        let is_up = is_link_up(&msg);

        // Check if this is a state change or new interface
        match link_states.get(&index) {
            Some((_existing_name, was_up)) => {
                // Existing interface - check for state change
                if is_up != *was_up {
                    if is_up {
                        log!("network", "Link up detected: {} (index {})", name, index);
                        let _ = tx
                            .send(NetworkEvent::LinkUp {
                                name: name.clone(),
                                index,
                            })
                            .await;
                    } else {
                        log!("network", "Link down detected: {} (index {})", name, index);
                        let _ = tx
                            .send(NetworkEvent::LinkDown {
                                name: name.clone(),
                                index,
                            })
                            .await;
                    }
                    link_states.insert(index, (name, is_up));
                }
            }
            None => {
                // New interface detected
                if let Some(mac_addr) = mac {
                    log!(
                        "network",
                        "New link added: {} (index {}, MAC {:02x}:{:02x}:{:02x}:{:02x}:{:02x}:{:02x})",
                        name,
                        index,
                        mac_addr[0],
                        mac_addr[1],
                        mac_addr[2],
                        mac_addr[3],
                        mac_addr[4],
                        mac_addr[5]
                    );
                    let _ = tx
                        .send(NetworkEvent::LinkAdded {
                            name: name.clone(),
                            index,
                            mac: mac_addr,
                        })
                        .await;
                }
                link_states.insert(index, (name, is_up));
            }
        }
    }
    Ok(())
}

/// Handle DelLink messages (link removed)
async fn handle_del_link(
    msg: LinkMessage,
    tx: &mpsc::Sender<NetworkEvent>,
    link_states: &mut HashMap<u32, (String, bool)>,
) -> Result<()> {
    if let Some((name, index, _)) = extract_link_info(&msg) {
        if !super::interface::is_ethernet_interface(&name) {
            return Ok(());
        }

        log!("network", "Link deleted: {} (index {})", name, index);
        let _ = tx.send(NetworkEvent::LinkDeleted { name, index }).await;
        link_states.remove(&index);
    }
    Ok(())
}
