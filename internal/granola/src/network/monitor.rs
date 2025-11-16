use crate::log;
use crate::network::netlink::link;
use anyhow::Result;
use futures::stream::StreamExt;
use netlink_packet_core::NetlinkPayload;
use netlink_packet_route::{RouteNetlinkMessage, link::LinkMessage};
use rtnetlink::Handle;
use std::collections::HashMap;
use tokio::sync::mpsc;

#[derive(Debug, Clone)]
#[allow(clippy::enum_variant_names)]
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

pub struct MonitorConfig {
    pub monitor_link_state: bool,
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

pub async fn start_monitor(
    _handle: Handle,
    config: MonitorConfig,
) -> Result<mpsc::Receiver<NetworkEvent>> {
    let (tx, rx) = mpsc::channel(32);

    let (connection, handle, mut messages) = rtnetlink::new_connection()?;

    tokio::spawn(async move {
        let _ = connection.await;
    });

    let mut link_states: HashMap<u32, (String, bool)> = HashMap::new();

    tokio::spawn(async move {
        if let Err(e) = initial_scan(&handle, &mut link_states).await {
            log!("network", "Initial interface scan failed: {}", e);
        }

        while let Some((message, _)) = messages.next().await {
            if let NetlinkPayload::InnerMessage(route_msg) = message.payload
                && let Err(e) = handle_message(route_msg, &tx, &config, &mut link_states).await
            {
                log!("network", "Error handling netlink message: {}", e);
            }
        }
    });

    log!("network", "Network event monitor started");
    Ok(rx)
}

async fn initial_scan(
    handle: &Handle,
    link_states: &mut HashMap<u32, (String, bool)>,
) -> Result<()> {
    use futures::stream::TryStreamExt;

    let mut links = handle.link().get().execute();
    while let Some(link_msg) = links.try_next().await? {
        if let Some((name, index, _)) = extract_link_info(&link_msg)
            && super::interface::is_ethernet_interface(&name)
        {
            let is_up = link::is_link_flag_up(&link_msg);
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
    Ok(())
}

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

fn extract_link_info(msg: &LinkMessage) -> Option<(String, u32, Option<[u8; 6]>)> {
    let index = msg.header.index;
    let name = link::extract_name_from_link(msg)?;
    let mac = link::extract_mac_from_link(msg);

    Some((name, index, mac))
}

async fn handle_new_link(
    msg: LinkMessage,
    tx: &mpsc::Sender<NetworkEvent>,
    link_states: &mut HashMap<u32, (String, bool)>,
) -> Result<()> {
    if let Some((name, index, mac)) = extract_link_info(&msg) {
        if !super::interface::is_ethernet_interface(&name) {
            return Ok(());
        }

        let is_up = link::is_link_flag_up(&msg);

        match link_states.get(&index) {
            Some((_existing_name, was_up)) => {
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
