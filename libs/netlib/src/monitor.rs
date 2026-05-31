//! Netlink-based monitor that watches for network interface state changes.

use std::collections::HashMap;
use std::io;

use rtnetlink::Handle;
use rtnetlink::packet_core::{NetlinkMessage, NetlinkPayload};
use rtnetlink::packet_route::{RouteNetlinkMessage, link::LinkFlags, link::LinkMessage};
use thiserror::Error;
use tokio::sync::mpsc;
use tokio_stream::StreamExt as _;

use crate::interface::{Name, is_ethernet};
use crate::link;
use crate::mac;

#[derive(Debug, Error)]
pub enum Failure {
    #[error("failed to create netlink connection: {0}")]
    Connection(#[source] io::Error),
    #[error("failed to enumerate links: {0}")]
    List(#[source] rtnetlink::Error),
}

pub type Result<T> = core::result::Result<T, Failure>;

/// An event emitted by the network monitor when a link changes state.
#[derive(Debug, Clone)]
pub enum Event {
    Up {
        name: Name,
        index: u32,
    },
    Down {
        name: Name,
        index: u32,
    },
    Added {
        name: Name,
        index: u32,
        mac: [u8; 6],
    },
    Deleted {
        name: Name,
        index: u32,
    },
}

/// Controls which categories of events the monitor emits.
pub struct Config {
    pub monitor_link_state: bool,
    pub monitor_link_changes: bool,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            monitor_link_state: true,
            monitor_link_changes: true,
        }
    }
}

/// Starts the network event monitor, returning a receiver of [`Event`] values.
///
/// # Errors
///
/// Returns an error when creating the netlink connection fails.
pub fn start(config: Config) -> Result<mpsc::Receiver<Event>> {
    let (tx, rx) = mpsc::channel(32);
    let (connection, handle, mut messages) =
        rtnetlink::new_connection().map_err(Failure::Connection)?;

    tokio::spawn(async move {
        let () = connection.await;
    });

    let mut link_states: HashMap<u32, (String, bool)> = HashMap::new();

    tokio::spawn(async move {
        if let Err(e) = initial_scan(&handle, &mut link_states).await {
            println!("Initial interface scan failed: {e}");
        }

        while let Some((message, _)) = messages.next().await {
            process_message(message, &tx, &config, &mut link_states).await;
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
    while let Some(link_msg) = links.try_next().await.map_err(Failure::List)? {
        let Some((name, index, _)) = extract_link_info(&link_msg) else {
            continue;
        };
        if !is_ethernet(&name) {
            continue;
        }
        let has_carrier = link_msg.header.flags.contains(LinkFlags::LowerUp);
        let is_admin_up = link_msg.header.flags.contains(LinkFlags::Up);
        link_states.insert(index, (name.clone(), has_carrier));
        println!(
            "Initial state: {name} (index {index}) = admin:{} carrier:{}",
            if is_admin_up { "up" } else { "down" },
            if has_carrier { "yes" } else { "no" }
        );
    }
    Ok(())
}

async fn process_message(
    message: NetlinkMessage<RouteNetlinkMessage>,
    tx: &mpsc::Sender<Event>,
    config: &Config,
    link_states: &mut HashMap<u32, (String, bool)>,
) {
    let NetlinkPayload::InnerMessage(route_msg) = message.payload else {
        return;
    };
    if let Err(e) = handle_message(route_msg, tx, config, link_states).await {
        println!("Error handling netlink message: {e}");
    }
}

async fn handle_message(
    msg: RouteNetlinkMessage,
    tx: &mpsc::Sender<Event>,
    config: &Config,
    link_states: &mut HashMap<u32, (String, bool)>,
) -> Result<()> {
    if let RouteNetlinkMessage::NewLink(link_msg) = msg {
        if config.monitor_link_changes || config.monitor_link_state {
            handle_new_link(link_msg, tx, link_states).await?;
        }
    } else if let RouteNetlinkMessage::DelLink(link_msg) = msg
        && config.monitor_link_changes
    {
        handle_del_link(link_msg, tx, link_states).await?;
    } else {
        return Ok(());
    }

    Ok(())
}

fn extract_link_info(msg: &LinkMessage) -> Option<(String, u32, Option<[u8; 6]>)> {
    let index = msg.header.index;
    let name = link::extract_name(msg)?;
    let mac = link::extract_mac(msg);
    Some((name, index, mac))
}

async fn handle_new_link(
    msg: LinkMessage,
    tx: &mpsc::Sender<Event>,
    link_states: &mut HashMap<u32, (String, bool)>,
) -> Result<()> {
    let Some((raw_name, index, mac)) = extract_link_info(&msg) else {
        return Ok(());
    };
    if !is_ethernet(&raw_name) {
        return Ok(());
    }

    let carrier_present = msg.header.flags.contains(LinkFlags::LowerUp);

    match link_states.get(&index) {
        Some(&(_, previous_carrier)) if carrier_present != previous_carrier => {
            let Some(name) = Name::new(raw_name.clone()).ok() else {
                return Ok(());
            };
            if carrier_present {
                println!("Carrier detected: {name} (index {index})");
                let _send_failed = tx.send(Event::Up { name, index }).await.is_err();
            } else {
                println!("Carrier lost: {name} (index {index})");
                let _send_failed = tx.send(Event::Down { name, index }).await.is_err();
            }
            link_states.insert(index, (raw_name, carrier_present));
        }
        Some(_) => {}
        None => emit_link_added(raw_name, index, mac, carrier_present, tx, link_states).await,
    }
    Ok(())
}

async fn emit_link_added(
    raw_name: String,
    index: u32,
    mac: Option<[u8; 6]>,
    carrier_present: bool,
    tx: &mpsc::Sender<Event>,
    link_states: &mut HashMap<u32, (String, bool)>,
) {
    if let Some(mac_addr) = mac {
        let Some(name) = Name::new(raw_name.clone()).ok() else {
            link_states.insert(index, (raw_name, carrier_present));
            return;
        };
        println!(
            "New link added: {name} (index {index}, MAC {})",
            mac::format(&mac_addr)
        );
        let _send_failed = tx
            .send(Event::Added {
                name,
                index,
                mac: mac_addr,
            })
            .await
            .is_err();
    }
    link_states.insert(index, (raw_name, carrier_present));
}

async fn handle_del_link(
    msg: LinkMessage,
    tx: &mpsc::Sender<Event>,
    link_states: &mut HashMap<u32, (String, bool)>,
) -> Result<()> {
    let Some((raw_name, index, _)) = extract_link_info(&msg) else {
        return Ok(());
    };
    if !is_ethernet(&raw_name) {
        return Ok(());
    }
    let Some(name) = Name::new(raw_name.clone()).ok() else {
        return Ok(());
    };
    println!("Link deleted: {name} (index {index})");
    let _send_failed = tx.send(Event::Deleted { name, index }).await.is_err();
    link_states.remove(&index);
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use rtnetlink::packet_core::{NetlinkHeader, NetlinkMessage, NetlinkPayload};
    use rtnetlink::packet_route::RouteNetlinkMessage;
    use rtnetlink::packet_route::link::{LinkAttribute, LinkFlags, LinkMessage};

    use super::*;

    fn link_message(
        name: Option<&str>,
        index: u32,
        mac: Option<[u8; 6]>,
        carrier: bool,
    ) -> LinkMessage {
        let mut link = LinkMessage::default();
        link.header.index = index;
        if carrier {
            link.header.flags.insert(LinkFlags::LowerUp);
        }
        if let Some(name) = name {
            link.attributes.push(LinkAttribute::IfName(name.to_owned()));
        }
        if let Some(mac) = mac {
            link.attributes.push(LinkAttribute::Address(mac.to_vec()));
        }
        link
    }

    fn config() -> Config {
        Config {
            monitor_link_state: true,
            monitor_link_changes: true,
        }
    }

    #[tokio::test]
    async fn handle_new_link_emits_up_on_carrier_gain() {
        // ARRANGE
        let (tx, mut rx) = mpsc::channel(1);
        let mut states = HashMap::from([(2, ("eth0".to_owned(), false))]);
        let link = link_message(Some("eth0"), 2, None, true);

        // ACT
        handle_new_link(link, &tx, &mut states)
            .await
            .expect("event should handle");
        let event = rx.recv().await.expect("event should be sent");

        // ASSERT
        assert!(matches!(event, Event::Up { ref name, index } if name == "eth0" && index == 2));
        assert_eq!(states.get(&2), Some(&("eth0".to_owned(), true)));
    }

    #[tokio::test]
    async fn handle_new_link_emits_down_on_carrier_loss() {
        // ARRANGE
        let (tx, mut rx) = mpsc::channel(1);
        let mut states = HashMap::from([(2, ("eth0".to_owned(), true))]);
        let link = link_message(Some("eth0"), 2, None, false);

        // ACT
        handle_new_link(link, &tx, &mut states)
            .await
            .expect("event should handle");
        let event = rx.recv().await.expect("event should be sent");

        // ASSERT
        assert!(matches!(event, Event::Down { ref name, index } if name == "eth0" && index == 2));
        assert_eq!(states.get(&2), Some(&("eth0".to_owned(), false)));
    }

    #[tokio::test]
    async fn handle_new_link_emits_added_for_unknown_ethernet_with_mac() {
        // ARRANGE
        let (tx, mut rx) = mpsc::channel(1);
        let mut states = HashMap::new();
        let mac = [1, 2, 3, 4, 5, 6];
        let link = link_message(Some("eth0"), 2, Some(mac), true);

        // ACT
        handle_new_link(link, &tx, &mut states)
            .await
            .expect("event should handle");
        let event = rx.recv().await.expect("event should be sent");

        // ASSERT
        assert!(matches!(event, Event::Added { ref name, index, mac: got }
            if name == "eth0" && index == 2 && got == mac));
        assert_eq!(states.get(&2), Some(&("eth0".to_owned(), true)));
    }

    #[tokio::test]
    async fn handle_new_link_tracks_unknown_ethernet_without_mac_without_event() {
        // ARRANGE
        let (tx, mut rx) = mpsc::channel(1);
        let mut states = HashMap::new();
        let link = link_message(Some("eth0"), 2, None, false);

        // ACT
        handle_new_link(link, &tx, &mut states)
            .await
            .expect("event should handle");

        // ASSERT
        rx.try_recv().unwrap_err();
        assert_eq!(states.get(&2), Some(&("eth0".to_owned(), false)));
    }

    #[tokio::test]
    async fn handle_new_link_ignores_non_ethernet_and_missing_names() {
        // ARRANGE
        let (tx, mut rx) = mpsc::channel(1);
        let mut states = HashMap::new();

        // ACT
        handle_new_link(link_message(Some("lo"), 1, None, true), &tx, &mut states)
            .await
            .expect("event should handle");
        handle_new_link(link_message(None, 2, None, true), &tx, &mut states)
            .await
            .expect("event should handle");

        // ASSERT
        rx.try_recv().unwrap_err();
        assert!(states.is_empty());
    }

    #[tokio::test]
    async fn handle_new_link_ignores_unchanged_carrier() {
        // ARRANGE
        let (tx, mut rx) = mpsc::channel(1);
        let mut states = HashMap::from([(2, ("eth0".to_owned(), true))]);
        let link = link_message(Some("eth0"), 2, None, true);

        // ACT
        handle_new_link(link, &tx, &mut states)
            .await
            .expect("event should handle");

        // ASSERT
        rx.try_recv().unwrap_err();
        assert_eq!(states.get(&2), Some(&("eth0".to_owned(), true)));
    }

    #[tokio::test]
    async fn handle_del_link_emits_deleted_and_removes_state() {
        // ARRANGE
        let (tx, mut rx) = mpsc::channel(1);
        let mut states = HashMap::from([(2, ("eth0".to_owned(), true))]);
        let link = link_message(Some("eth0"), 2, None, false);

        // ACT
        handle_del_link(link, &tx, &mut states)
            .await
            .expect("event should handle");
        let event = rx.recv().await.expect("event should be sent");

        // ASSERT
        assert!(
            matches!(event, Event::Deleted { ref name, index } if name == "eth0" && index == 2)
        );
        assert!(!states.contains_key(&2));
    }

    #[tokio::test]
    async fn handle_del_link_ignores_non_ethernet_and_missing_names() {
        // ARRANGE
        let (tx, mut rx) = mpsc::channel(1);
        let mut states = HashMap::from([(1, ("lo".to_owned(), true))]);

        // ACT
        handle_del_link(link_message(Some("lo"), 1, None, false), &tx, &mut states)
            .await
            .expect("event should handle");
        handle_del_link(link_message(None, 2, None, false), &tx, &mut states)
            .await
            .expect("event should handle");

        // ASSERT
        rx.try_recv().unwrap_err();
        assert!(states.contains_key(&1));
    }

    #[tokio::test]
    async fn handle_message_respects_config_flags() {
        // ARRANGE
        let (tx, mut rx) = mpsc::channel(1);
        let mut states = HashMap::from([(2, ("eth0".to_owned(), false))]);
        let disabled = Config {
            monitor_link_state: false,
            monitor_link_changes: false,
        };

        // ACT
        handle_message(
            RouteNetlinkMessage::NewLink(link_message(Some("eth0"), 2, None, true)),
            &tx,
            &disabled,
            &mut states,
        )
        .await
        .expect("event should handle");

        // ASSERT
        rx.try_recv().unwrap_err();
        assert_eq!(states.get(&2), Some(&("eth0".to_owned(), false)));
    }

    #[tokio::test]
    async fn process_message_dispatches_inner_messages_only() {
        // ARRANGE
        let (tx, mut rx) = mpsc::channel(1);
        let mut states = HashMap::from([(2, ("eth0".to_owned(), false))]);
        let message = NetlinkMessage::new(
            NetlinkHeader::default(),
            NetlinkPayload::InnerMessage(RouteNetlinkMessage::NewLink(link_message(
                Some("eth0"),
                2,
                None,
                true,
            ))),
        );

        // ACT
        process_message(message, &tx, &config(), &mut states).await;
        let event = rx.recv().await.expect("event should be sent");

        // ASSERT
        assert!(matches!(event, Event::Up { ref name, index } if name == "eth0" && index == 2));
    }

    #[tokio::test]
    async fn process_message_ignores_non_inner_payloads() {
        // ARRANGE
        let (tx, mut rx) = mpsc::channel(1);
        let mut states = HashMap::new();
        let message = NetlinkMessage::new(NetlinkHeader::default(), NetlinkPayload::Noop);

        // ACT
        process_message(message, &tx, &config(), &mut states).await;

        // ASSERT
        rx.try_recv().unwrap_err();
    }

    #[test]
    fn config_default_enables_state_and_change_events() {
        // ACT
        let config = Config::default();

        // ASSERT
        assert!(config.monitor_link_state);
        assert!(config.monitor_link_changes);
    }
}
