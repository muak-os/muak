//! Netlink message pump that turns link notifications into monitor events.

use std::collections::HashMap;

use netlink::link::device;
use netlink::link::ifname::Name;
use rtnetlink::packet_core::{NetlinkMessage, NetlinkPayload};
use rtnetlink::packet_route::RouteNetlinkMessage;
use rtnetlink::packet_route::link::{LinkFlags, LinkMessage};
use rtnetlink::sys::SocketAddr;
use tokio::sync::mpsc;
use tokio_stream::{Stream, StreamExt as _};

use crate::{Config, Event, Failure};

pub(super) async fn run<S>(
    config: &Config,
    messages: &mut S,
    tx: &mpsc::Sender<Event>,
    mut link_states: HashMap<u32, bool>,
) where
    S: Stream<Item = (NetlinkMessage<RouteNetlinkMessage>, SocketAddr)> + Unpin,
{
    while let Some((message, _)) = messages.next().await {
        process_message(message, tx, config, &mut link_states).await;
    }
    let _send_failed = tx.send(Event::Error(Failure::Closed)).await.is_err();
}

async fn process_message(
    message: NetlinkMessage<RouteNetlinkMessage>,
    tx: &mpsc::Sender<Event>,
    config: &Config,
    link_states: &mut HashMap<u32, bool>,
) {
    match message.payload {
        NetlinkPayload::InnerMessage(route_msg) => {
            handle_message(route_msg, tx, config, link_states).await;
        }
        NetlinkPayload::Overrun(_) => report(tx, Failure::Overrun),
        NetlinkPayload::Error(error) => {
            if let Some(code) = error.code {
                report(tx, Failure::Kernel(code.abs()));
            }
        }
        NetlinkPayload::Done(_) | NetlinkPayload::Noop => (),
        _ => {}
    }
}

async fn handle_message(
    msg: RouteNetlinkMessage,
    tx: &mpsc::Sender<Event>,
    config: &Config,
    link_states: &mut HashMap<u32, bool>,
) {
    if !config.monitor_link_changes && !config.monitor_link_state {
        return;
    }

    if let RouteNetlinkMessage::NewLink(link_msg) = msg {
        handle_new_link(link_msg, tx, config, link_states).await;
        return;
    }
    if let RouteNetlinkMessage::DelLink(link_msg) = msg {
        handle_del_link(link_msg, tx, config, link_states).await;
    }
}

async fn handle_new_link(
    msg: LinkMessage,
    tx: &mpsc::Sender<Event>,
    config: &Config,
    link_states: &mut HashMap<u32, bool>,
) {
    let Some((raw_name, index, mac)) = extract_link_info(&msg) else {
        return;
    };
    if !device::is_ethernet(&raw_name) {
        return;
    }
    if device::is_virtual(&msg) {
        return;
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
        return;
    };
    if carrier_present == previous_carrier {
        return;
    }
    if !config.monitor_link_state {
        link_states.insert(index, carrier_present);
        return;
    }
    let Some(name) = Name::new(raw_name).ok() else {
        return;
    };
    link_states.insert(index, carrier_present);
    if carrier_present {
        let _send_failed = tx.send(Event::Up { name, index }).await.is_err();
    } else {
        let _send_failed = tx.send(Event::Down { name, index }).await.is_err();
    }
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
) {
    link_states.remove(&msg.header.index);

    let Some((raw_name, index, _)) = extract_link_info(&msg) else {
        return;
    };
    if !device::is_ethernet(&raw_name) {
        return;
    }
    if device::is_virtual(&msg) {
        return;
    }
    if !config.monitor_link_changes {
        return;
    }
    let Some(name) = Name::new(raw_name).ok() else {
        return;
    };
    let _send_failed = tx.send(Event::Deleted { name, index }).await.is_err();
}

fn report(tx: &mpsc::Sender<Event>, failure: Failure) {
    let _dropped = tx.try_send(Event::Error(failure)).is_err();
}

fn extract_link_info(msg: &LinkMessage) -> Option<(String, u32, Option<[u8; 6]>)> {
    let index = msg.header.index;
    let name = device::extract_name(msg)?;
    let mac = device::extract_mac(msg);

    Some((name, index, mac))
}

#[cfg(test)]
mod tests {
    use core::num::NonZeroI32;

    use rtnetlink::packet_core::{ErrorBuffer, ErrorMessage, NetlinkHeader, Parseable as _};
    use rtnetlink::packet_route::link::{InfoKind, LinkAttribute, LinkInfo};

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

    fn virtual_link_message(name: &str, index: u32, mac: [u8; 6], carrier: bool) -> LinkMessage {
        let mut link = link_message(Some(name), index, Some(mac), carrier);
        link.attributes
            .push(LinkAttribute::LinkInfo(vec![LinkInfo::Kind(
                InfoKind::Veth,
            )]));
        link
    }

    fn config() -> Config {
        Config {
            monitor_link_state: true,
            monitor_link_changes: true,
        }
    }

    fn error_message(code: Option<NonZeroI32>) -> ErrorMessage {
        let mut raw = [0_u8; 4];
        let mut buffer = ErrorBuffer::new(&mut raw);
        buffer.set_code(code.map_or(0_i32, NonZeroI32::get));
        let checked = ErrorBuffer::new_checked(&raw).expect("4 byte buffer");
        ErrorMessage::parse(&checked).expect("valid error message")
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
        handle_new_link(link, &tx, &config(), &mut states).await;
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
        handle_new_link(link, &tx, &config(), &mut states).await;
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
        handle_new_link(link, &tx, &config(), &mut states).await;
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
        handle_new_link(link, &tx, &config(), &mut states).await;

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
        .await;
        handle_new_link(
            link_message(None, 2, None, true),
            &tx,
            &config(),
            &mut states,
        )
        .await;

        // ASSERT
        rx.try_recv().unwrap_err();
        assert!(states.is_empty());
    }

    #[tokio::test]
    async fn handle_new_link_ignores_virtual_ethernet_and_seeds_no_state() {
        // ARRANGE
        let (tx, mut rx) = mpsc::channel(1);
        let mut states = HashMap::new();
        let link = virtual_link_message("eth0", 2, [1, 2, 3, 4, 5, 6], true);

        // ACT
        handle_new_link(link, &tx, &config(), &mut states).await;

        // ASSERT
        rx.try_recv().unwrap_err();
        assert!(
            states.is_empty(),
            "a virtual link must not seed link state, so it cannot emit Up or Down later"
        );
    }

    #[tokio::test]
    async fn handle_new_link_ignores_unchanged_carrier() {
        // ARRANGE
        let (tx, mut rx) = mpsc::channel(1);
        let mut states = HashMap::from([(2, true)]);
        let link = link_message(Some("eth0"), 2, None, true);

        // ACT
        handle_new_link(link, &tx, &config(), &mut states).await;

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
        handle_del_link(link, &tx, &config(), &mut states).await;
        let event = rx.recv().await.expect("event should be sent");

        // ASSERT
        assert!(
            matches!(event, Event::Deleted { ref name, index } if name == "eth0" && index == 2)
        );
        assert!(!states.contains_key(&2));
    }

    #[tokio::test]
    async fn handle_del_link_emits_no_event_for_non_ethernet_and_missing_names() {
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
        .await;
        handle_del_link(
            link_message(None, 2, None, false),
            &tx,
            &config(),
            &mut states,
        )
        .await;

        // ASSERT
        rx.try_recv().unwrap_err();
        assert!(
            !states.contains_key(&1),
            "state is dropped before the ethernet filter, so a renamed link leaves nothing behind"
        );
    }

    #[tokio::test]
    async fn handle_del_link_emits_no_event_for_virtual_ethernet() {
        // ARRANGE
        let (tx, mut rx) = mpsc::channel(1);
        let mut states = HashMap::from([(2, true)]);
        let link = virtual_link_message("eth0", 2, [1, 2, 3, 4, 5, 6], true);

        // ACT
        handle_del_link(link, &tx, &config(), &mut states).await;

        // ASSERT
        rx.try_recv().unwrap_err();
        assert!(!states.contains_key(&2));
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
        .await;
        handle_message(
            RouteNetlinkMessage::NewLink(link_message(Some("eth0"), 2, Some(mac), false)),
            &tx,
            &config(),
            &mut states,
        )
        .await;
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
        .await;
        handle_message(
            RouteNetlinkMessage::NewLink(link_message(Some("eth0"), 2, None, true)),
            &tx,
            &state_only(),
            &mut states,
        )
        .await;
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
        .await;
        handle_message(
            RouteNetlinkMessage::NewLink(link_message(Some("eth0"), 2, None, true)),
            &tx,
            &changes_only(),
            &mut states,
        )
        .await;
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
        .await;

        // ASSERT
        rx.try_recv().unwrap_err();
        assert_eq!(states.get(&2), Some(&false));
    }

    #[tokio::test]
    async fn process_message_dispatches_inner_link_messages() {
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

    #[tokio::test]
    async fn process_message_reports_overrun() {
        // ARRANGE
        let (tx, mut rx) = mpsc::channel(1);
        let mut states = HashMap::new();
        let message = NetlinkMessage::new(
            NetlinkHeader::default(),
            NetlinkPayload::Overrun(Vec::new()),
        );

        // ACT
        process_message(message, &tx, &config(), &mut states).await;
        let event = rx.try_recv().expect("event should be sent");

        // ASSERT
        assert!(matches!(event, Event::Error(Failure::Overrun)));
    }

    #[tokio::test]
    async fn process_message_reports_kernel_error() {
        // ARRANGE
        let (tx, mut rx) = mpsc::channel(1);
        let mut states = HashMap::new();
        let message = NetlinkMessage::new(
            NetlinkHeader::default(),
            NetlinkPayload::Error(error_message(NonZeroI32::new(-105_i32))),
        );

        // ACT
        process_message(message, &tx, &config(), &mut states).await;
        let event = rx.try_recv().expect("event should be sent");

        // ASSERT
        assert!(matches!(event, Event::Error(Failure::Kernel(code)) if code.get() == 105_i32));
    }

    #[tokio::test]
    async fn process_message_ignores_ack_errors() {
        // ARRANGE
        let (tx, mut rx) = mpsc::channel(1);
        let mut states = HashMap::new();
        let message = NetlinkMessage::new(
            NetlinkHeader::default(),
            NetlinkPayload::Error(error_message(None)),
        );

        // ACT
        process_message(message, &tx, &config(), &mut states).await;

        // ASSERT
        rx.try_recv().unwrap_err();
    }

    #[tokio::test]
    async fn run_reports_closed_when_stream_ends() {
        // ARRANGE
        let (tx, mut rx) = mpsc::channel(1);

        // ACT
        run(&config(), &mut tokio_stream::empty(), &tx, HashMap::new()).await;
        let event = rx.try_recv().expect("event should be sent");

        // ASSERT
        assert!(matches!(event, Event::Error(Failure::Closed)));
    }
}
