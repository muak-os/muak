//! Netlink-based monitor that watches for network interface state changes.

use std::collections::HashMap;

use anyhow::Result;
use netlib::interface::{InterfaceName, is_ethernet};
use rtnetlink::Handle;
use rtnetlink::packet_core::NetlinkPayload;
use rtnetlink::packet_route::{RouteNetlinkMessage, link::LinkFlags, link::LinkMessage};
use tokio::sync::mpsc;
use tokio_stream::StreamExt;

#[derive(Debug, Clone)]
#[allow(clippy::enum_variant_names)]
pub enum NetworkEvent {
    LinkUp {
        name: InterfaceName,
        index: u32,
    },
    LinkDown {
        name: InterfaceName,
        index: u32,
    },
    LinkAdded {
        name: InterfaceName,
        index: u32,
        mac: [u8; 6],
    },
    LinkDeleted {
        name: InterfaceName,
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
            process_netlink_message(message, &tx, &config, &mut link_states).await;
        }
    });

    println!("Network event monitor started");
    Ok(rx)
}

async fn process_netlink_message(
    message: rtnetlink::packet_core::NetlinkMessage<RouteNetlinkMessage>,
    tx: &mpsc::Sender<NetworkEvent>,
    config: &MonitorConfig,
    link_states: &mut HashMap<u32, (String, bool)>,
) {
    let NetlinkPayload::InnerMessage(route_msg) = message.payload else {
        return;
    };
    if let Err(e) = handle_message(route_msg, tx, config, link_states).await {
        eprintln!("Error handling netlink message: {}", e);
    }
}

async fn initial_scan(
    handle: &Handle,
    link_states: &mut HashMap<u32, (String, bool)>,
) -> Result<()> {
    let mut links = handle.link().get().execute();
    while let Some(link_msg) = links.try_next().await? {
        if let Some((name, index, _)) = extract_link_info(&link_msg)
            && is_ethernet(&name)
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
        _ => {}
    }
    Ok(())
}

fn extract_link_info(msg: &LinkMessage) -> Option<(String, u32, Option<[u8; 6]>)> {
    let index = msg.header.index;
    let name = netlib::link::extract_name(msg)?;
    let mac = netlib::link::extract_mac(msg);

    Some((name, index, mac))
}

async fn handle_new_link(
    msg: LinkMessage,
    tx: &mpsc::Sender<NetworkEvent>,
    link_states: &mut HashMap<u32, (String, bool)>,
) -> Result<()> {
    let Some((raw_name, index, mac)) = extract_link_info(&msg) else {
        return Ok(());
    };
    if !is_ethernet(&raw_name) {
        return Ok(());
    }

    let has_carrier = msg.header.flags.contains(LinkFlags::LowerUp);

    match link_states.get(&index) {
        Some((_existing_name, had_carrier)) if has_carrier != *had_carrier => {
            let Some(name) = InterfaceName::new(&raw_name).ok() else {
                return Ok(());
            };
            if has_carrier {
                println!("Carrier detected: {} (index {})", name, index);
                let _ = tx.send(NetworkEvent::LinkUp { name, index }).await;
            } else {
                println!("Carrier lost: {} (index {})", name, index);
                let _ = tx.send(NetworkEvent::LinkDown { name, index }).await;
            }
            link_states.insert(index, (raw_name, has_carrier));
        }
        Some(_) => {}
        None => {
            emit_link_added(raw_name.clone(), index, mac, has_carrier, tx, link_states).await;
        }
    }
    Ok(())
}

async fn emit_link_added(
    raw_name: String,
    index: u32,
    mac: Option<[u8; 6]>,
    has_carrier: bool,
    tx: &mpsc::Sender<NetworkEvent>,
    link_states: &mut HashMap<u32, (String, bool)>,
) {
    if let Some(mac_addr) = mac {
        let Some(name) = InterfaceName::new(&raw_name).ok() else {
            link_states.insert(index, (raw_name, has_carrier));
            return;
        };
        println!(
            "New link added: {} (index {}, MAC {})",
            name,
            index,
            netlib::mac::format(&mac_addr)
        );
        let _ = tx
            .send(NetworkEvent::LinkAdded {
                name,
                index,
                mac: mac_addr,
            })
            .await;
    }
    link_states.insert(index, (raw_name, has_carrier));
}

async fn handle_del_link(
    msg: LinkMessage,
    tx: &mpsc::Sender<NetworkEvent>,
    link_states: &mut HashMap<u32, (String, bool)>,
) -> Result<()> {
    if let Some((raw_name, index, _)) = extract_link_info(&msg) {
        if !is_ethernet(&raw_name) {
            return Ok(());
        }
        let Some(name) = InterfaceName::new(&raw_name).ok() else {
            return Ok(());
        };
        println!("Link deleted: {} (index {})", name, index);
        let _ = tx.send(NetworkEvent::LinkDeleted { name, index }).await;
        link_states.remove(&index);
    }
    Ok(())
}
