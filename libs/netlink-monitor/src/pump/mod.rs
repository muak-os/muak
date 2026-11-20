//! Netlink message pump that turns link notifications into monitor events.

mod scan;

use std::collections::HashMap;

use netlink::link::{Name, is_ethernet};
use rtnetlink::Handle;
use rtnetlink::packet_core::{NetlinkMessage, NetlinkPayload};
use rtnetlink::packet_route::RouteNetlinkMessage;
use rtnetlink::packet_route::link::{LinkFlags, LinkMessage};
use rtnetlink::sys::SocketAddr;
use tokio::sync::mpsc;
use tokio_stream::{Stream, StreamExt as _};

use crate::{Config, Event, Result};

pub(super) async fn run<S>(
    config: &Config,
    handle: &Handle,
    messages: &mut S,
    tx: &mpsc::Sender<Event>,
) where
    S: Stream<Item = (NetlinkMessage<RouteNetlinkMessage>, SocketAddr)> + Unpin,
{
    let mut link_states: HashMap<u32, bool> = HashMap::new();

    if let Err(e) = scan::initial_scan(handle, &mut link_states).await {
        println!("Initial interface scan failed: {e}");
    }

    while let Some((message, _)) = messages.next().await {
        process_message(message, tx, config, &mut link_states).await;
    }
}

async fn process_message(
    message: NetlinkMessage<RouteNetlinkMessage>,
    tx: &mpsc::Sender<Event>,
    config: &Config,
    link_states: &mut HashMap<u32, bool>,
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
    link_states: &mut HashMap<u32, bool>,
) -> Result<()> {
    if !config.monitor_link_changes && !config.monitor_link_state {
        return Ok(());
    }

    if let RouteNetlinkMessage::NewLink(link_msg) = msg {
        handle_new_link(link_msg, tx, config, link_states).await?;
    } else if let RouteNetlinkMessage::DelLink(link_msg) = msg {
        handle_del_link(link_msg, tx, config, link_states).await?;
    } else {
        return Ok(());
    }

    Ok(())
}

async fn handle_new_link(
    msg: LinkMessage,
    tx: &mpsc::Sender<Event>,
    config: &Config,
    link_states: &mut HashMap<u32, bool>,
) -> Result<()> {
    let Some((raw_name, index, mac)) = scan::extract_link_info(&msg) else {
        return Ok(());
    };
    if !is_ethernet(&raw_name) {
        return Ok(());
    }

    let carrier_present = msg.header.flags.contains(LinkFlags::LowerUp);

    let Some(previous_carrier) = link_states.get(&index).copied() else {
        emit_link_added(
            raw_name,
            index,
            mac,
            carrier_present,
            tx,
            config,
            link_states,
        )
        .await;
        return Ok(());
    };
    if carrier_present == previous_carrier {
        return Ok(());
    }
    if !config.monitor_link_state {
        link_states.insert(index, carrier_present);
        return Ok(());
    }
    let Some(name) = Name::new(raw_name).ok() else {
        return Ok(());
    };
    link_states.insert(index, carrier_present);
    if carrier_present {
        println!("Carrier detected: {name} (index {index})");
        let _send_failed = tx.send(Event::Up { name, index }).await.is_err();
    } else {
        println!("Carrier lost: {name} (index {index})");
        let _send_failed = tx.send(Event::Down { name, index }).await.is_err();
    }
    Ok(())
}

async fn emit_link_added(
    raw_name: String,
    index: u32,
    mac: Option<[u8; 6]>,
    carrier_present: bool,
    tx: &mpsc::Sender<Event>,
    config: &Config,
    link_states: &mut HashMap<u32, bool>,
) {
    link_states.insert(index, carrier_present);
    if !config.monitor_link_changes {
        return;
    }
    let Some(mac_addr) = mac else {
        return;
    };
    let Some(name) = Name::new(raw_name).ok() else {
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

async fn handle_del_link(
    msg: LinkMessage,
    tx: &mpsc::Sender<Event>,
    config: &Config,
    link_states: &mut HashMap<u32, bool>,
) -> Result<()> {
    let Some((raw_name, index, _)) = scan::extract_link_info(&msg) else {
        return Ok(());
    };
    if !is_ethernet(&raw_name) {
        return Ok(());
    }
    link_states.remove(&index);
    if !config.monitor_link_changes {
        return Ok(());
    }
    let Some(name) = Name::new(raw_name).ok() else {
        return Ok(());
    };
    println!("Link deleted: {name} (index {index})");
    let _send_failed = tx.send(Event::Deleted { name, index }).await.is_err();
    Ok(())
}

#[cfg(test)]
mod tests {
    use rtnetlink::packet_core::NetlinkHeader;
    use rtnetlink::packet_route::link::LinkAttribute;

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

    fn state_only() -> Config {
        Config {
            monitor_link_state: true,
            monitor_link_changes: false,
        }
    }

    fn changes_only() -> Config {
        Config {
            monitor_link_state: false,
            monitor_link_changes: true,
        }
    }

    #[tokio::test]
    async fn handle_new_link_emits_up_on_carrier_gain() {
        // ARRANGE
        let (tx, mut rx) = mpsc::channel(1);
        let mut states = HashMap::from([(2, false)]);
        let link = link_message(Some("eth0"), 2, None, true);

        // ACT
        handle_new_link(link, &tx, &config(), &mut states)
            .await
            .expect("event should handle");
        let event = rx.recv().await.expect("event should be sent");

        // ASSERT
        assert!(matches!(event, Event::Up { ref name, index } if name == "eth0" && index == 2));
        assert_eq!(states.get(&2), Some(&true));
    }

    #[tokio::test]
    async fn handle_new_link_emits_down_on_carrier_loss() {
        // ARRANGE
        let (tx, mut rx) = mpsc::channel(1);
        let mut states = HashMap::from([(2, true)]);
        let link = link_message(Some("eth0"), 2, None, false);

        // ACT
        handle_new_link(link, &tx, &config(), &mut states)
            .await
            .expect("event should handle");
        let event = rx.recv().await.expect("event should be sent");

        // ASSERT
        assert!(matches!(event, Event::Down { ref name, index } if name == "eth0" && index == 2));
        assert_eq!(states.get(&2), Some(&false));
    }

    #[tokio::test]
    async fn handle_new_link_emits_added_for_unknown_ethernet_with_mac() {
        // ARRANGE
        let (tx, mut rx) = mpsc::channel(1);
        let mut states = HashMap::new();
        let mac = [1, 2, 3, 4, 5, 6];
        let link = link_message(Some("eth0"), 2, Some(mac), true);

        // ACT
        handle_new_link(link, &tx, &config(), &mut states)
            .await
            .expect("event should handle");
        let event = rx.recv().await.expect("event should be sent");

        // ASSERT
        assert!(matches!(event, Event::Added { ref name, index, mac: got }
            if name == "eth0" && index == 2 && got == mac));
        assert_eq!(states.get(&2), Some(&true));
    }

    #[tokio::test]
    async fn handle_new_link_tracks_unknown_ethernet_without_mac_without_event() {
        // ARRANGE
        let (tx, mut rx) = mpsc::channel(1);
        let mut states = HashMap::new();
        let link = link_message(Some("eth0"), 2, None, false);

        // ACT
        handle_new_link(link, &tx, &config(), &mut states)
            .await
            .expect("event should handle");

        // ASSERT
        rx.try_recv().unwrap_err();
        assert_eq!(states.get(&2), Some(&false));
    }

    #[tokio::test]
    async fn handle_new_link_ignores_non_ethernet_and_missing_names() {
        // ARRANGE
        let (tx, mut rx) = mpsc::channel(1);
        let mut states = HashMap::new();

        // ACT
        handle_new_link(
            link_message(Some("lo"), 1, None, true),
            &tx,
            &config(),
            &mut states,
        )
        .await
        .expect("event should handle");
        handle_new_link(
            link_message(None, 2, None, true),
            &tx,
            &config(),
            &mut states,
        )
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
        let mut states = HashMap::from([(2, true)]);
        let link = link_message(Some("eth0"), 2, None, true);

        // ACT
        handle_new_link(link, &tx, &config(), &mut states)
            .await
            .expect("event should handle");

        // ASSERT
        rx.try_recv().unwrap_err();
        assert_eq!(states.get(&2), Some(&true));
    }

    #[tokio::test]
    async fn handle_del_link_emits_deleted_and_removes_state() {
        // ARRANGE
        let (tx, mut rx) = mpsc::channel(1);
        let mut states = HashMap::from([(2, true)]);
        let link = link_message(Some("eth0"), 2, None, false);

        // ACT
        handle_del_link(link, &tx, &config(), &mut states)
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
        let mut states = HashMap::from([(1, true)]);

        // ACT
        handle_del_link(
            link_message(Some("lo"), 1, None, false),
            &tx,
            &config(),
            &mut states,
        )
        .await
        .expect("event should handle");
        handle_del_link(
            link_message(None, 2, None, false),
            &tx,
            &config(),
            &mut states,
        )
        .await
        .expect("event should handle");

        // ASSERT
        rx.try_recv().unwrap_err();
        assert!(states.contains_key(&1));
    }

    #[tokio::test]
    async fn handle_message_removes_state_on_delete_when_link_changes_disabled() {
        // ARRANGE
        let (tx, mut rx) = mpsc::channel(2);
        let mut states = HashMap::from([(2, true)]);
        let mac = [1, 2, 3, 4, 5, 6];

        // ACT
        handle_message(
            RouteNetlinkMessage::DelLink(link_message(Some("eth0"), 2, None, true)),
            &tx,
            &state_only(),
            &mut states,
        )
        .await
        .expect("event should handle");
        handle_message(
            RouteNetlinkMessage::NewLink(link_message(Some("eth0"), 2, Some(mac), false)),
            &tx,
            &config(),
            &mut states,
        )
        .await
        .expect("event should handle");
        let event = rx.recv().await.expect("event should be sent");

        // ASSERT
        assert!(matches!(event, Event::Added { ref name, index, mac: got }
            if name == "eth0" && index == 2 && got == mac));
        assert_eq!(states.get(&2), Some(&false));
    }

    #[tokio::test]
    async fn handle_message_records_state_without_added_when_changes_disabled() {
        // ARRANGE
        let (tx, mut rx) = mpsc::channel(2);
        let mut states = HashMap::new();
        let mac = [1, 2, 3, 4, 5, 6];

        // ACT
        handle_message(
            RouteNetlinkMessage::NewLink(link_message(Some("eth0"), 2, Some(mac), false)),
            &tx,
            &state_only(),
            &mut states,
        )
        .await
        .expect("event should handle");
        handle_message(
            RouteNetlinkMessage::NewLink(link_message(Some("eth0"), 2, None, true)),
            &tx,
            &state_only(),
            &mut states,
        )
        .await
        .expect("event should handle");
        let event = rx.recv().await.expect("event should be sent");

        // ASSERT
        assert!(matches!(event, Event::Up { ref name, index } if name == "eth0" && index == 2));
        rx.try_recv().unwrap_err();
        assert_eq!(states.get(&2), Some(&true));
    }

    #[tokio::test]
    async fn handle_message_skips_state_events_when_link_state_disabled() {
        // ARRANGE
        let (tx, mut rx) = mpsc::channel(2);
        let mut states = HashMap::new();
        let mac = [1, 2, 3, 4, 5, 6];

        // ACT
        handle_message(
            RouteNetlinkMessage::NewLink(link_message(Some("eth0"), 2, Some(mac), false)),
            &tx,
            &changes_only(),
            &mut states,
        )
        .await
        .expect("event should handle");
        handle_message(
            RouteNetlinkMessage::NewLink(link_message(Some("eth0"), 2, None, true)),
            &tx,
            &changes_only(),
            &mut states,
        )
        .await
        .expect("event should handle");
        let event = rx.recv().await.expect("event should be sent");

        // ASSERT
        assert!(matches!(event, Event::Added { ref name, index, mac: got }
            if name == "eth0" && index == 2 && got == mac));
        rx.try_recv().unwrap_err();
        assert_eq!(states.get(&2), Some(&true));
    }

    #[tokio::test]
    async fn handle_message_respects_config_flags() {
        // ARRANGE
        let (tx, mut rx) = mpsc::channel(1);
        let mut states = HashMap::from([(2, false)]);
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
        assert_eq!(states.get(&2), Some(&false));
    }

    #[tokio::test]
    async fn process_message_dispatches_inner_messages_only() {
        // ARRANGE
        let (tx, mut rx) = mpsc::channel(1);
        let mut states = HashMap::from([(2, false)]);
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
}
