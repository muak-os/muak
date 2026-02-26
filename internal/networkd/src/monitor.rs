use std::collections::HashMap;

use anyhow::Result;
use rtnetlink::Handle;
use rtnetlink::packet_core::NetlinkPayload;
use rtnetlink::packet_route::{RouteNetlinkMessage, link::LinkFlags, link::LinkMessage};
use tokio::sync::mpsc;
use tokio_stream::StreamExt;

use crate::netlink::link;

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
            eprintln!("Initial interface scan failed: {}", e);
        }

        while let Some((message, _)) = messages.next().await {
            let NetlinkPayload::InnerMessage(route_msg) = message.payload else {
                continue;
            };
            let result = handle_message(route_msg, &tx, &config, &mut link_states).await;
            if let Err(e) = result {
                eprintln!("Error handling netlink message: {}", e);
            }
        }
    });

    println!("Network event monitor started");
    Ok(rx)
}

async fn initial_scan(
    handle: &Handle,
    link_states: &mut HashMap<u32, (String, bool)>,
) -> Result<()> {
    let mut links = handle.link().get().execute();
    while let Some(link_msg) = links.try_next().await? {
        if let Some((name, index, _)) = extract_link_info(&link_msg)
            && crate::interface::is_ethernet_interface(&name)
        {
            let has_carrier = link_msg.header.flags.contains(LinkFlags::LowerUp);
            let is_admin_up = link_msg.header.flags.contains(LinkFlags::Up);
            link_states.insert(index, (name.clone(), has_carrier));
            println!(
                "Initial state: {} (index {}) = admin:{} carrier:{}",
                name,
                index,
                if is_admin_up { "up" } else { "down" },
                if has_carrier { "yes" } else { "no" }
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
    let Some((name, index, mac)) = extract_link_info(&msg) else {
        return Ok(());
    };
    if !crate::interface::is_ethernet_interface(&name) {
        return Ok(());
    }

    let has_carrier = msg.header.flags.contains(LinkFlags::LowerUp);

    match link_states.get(&index) {
        Some((_existing_name, had_carrier)) if has_carrier != *had_carrier => {
            if has_carrier {
                println!("Carrier detected: {} (index {})", name, index);
                let _ = tx
                    .send(NetworkEvent::LinkUp {
                        name: name.to_string(),
                        index,
                    })
                    .await;
            } else {
                println!("Carrier lost: {} (index {})", name, index);
                let _ = tx
                    .send(NetworkEvent::LinkDown {
                        name: name.to_string(),
                        index,
                    })
                    .await;
            }
            link_states.insert(index, (name, has_carrier));
        }
        Some(_) => {}
        None => {
            if let Some(mac_addr) = mac {
                println!(
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
                        name: name.to_string(),
                        index,
                        mac: mac_addr,
                    })
                    .await;
            }
            link_states.insert(index, (name, has_carrier));
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
        if !crate::interface::is_ethernet_interface(&name) {
            return Ok(());
        }

        println!("Link deleted: {} (index {})", name, index);
        let _ = tx.send(NetworkEvent::LinkDeleted { name, index }).await;
        link_states.remove(&index);
    }
    Ok(())
}
