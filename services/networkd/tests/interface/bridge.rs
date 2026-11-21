//! Integration tests for bridge configuration on interface actor.

use core::net::Ipv4Addr;
use core::time::Duration;
use std::time::SystemTime;

use ::dhcp_client::lease::Lease;
use netlink::address::IpConfig;
use netlink::link::state::State;
use networkd::interface::lifecycle::DhcpState;
use tokio::sync::oneshot;

use super::*;

fn make_lease() -> Lease {
    Lease {
        obtained_at: SystemTime::now(),
        lease_time: Duration::from_hours(1),
        renewal_time: Duration::from_mins(30),
        rebind_time: Duration::from_secs(3150),
        server_ip: Ipv4Addr::new(192, 168, 1, 1),
        assigned_ip: Ipv4Addr::new(192, 168, 1, 100),
        prefix_len: 24,
        gateway: Some(Ipv4Addr::new(192, 168, 1, 1)),
        dns_servers: vec![Ipv4Addr::new(8, 8, 8, 8)],
    }
}

fn make_configured_snapshot(name: &str, index: u32, mac: [u8; 6]) -> Snapshot {
    Snapshot {
        name: Name::new(name).expect("valid name"),
        state: Lifecycle::Configured,
        index,
        mac,
        link: State::Up,
        ip: Some(IpConfig {
            address: Ipv4Addr::new(192, 168, 1, 100),
            prefix_len: 24,
            gateway: Some(Ipv4Addr::new(192, 168, 1, 1)),
            dns: vec![Ipv4Addr::new(8, 8, 8, 8)],
        }),
        lease: Some(make_lease()),
        dhcp_state: Some(DhcpState::Bound),
        ipv6: None,
        l3_owner: Name::new(name).expect("valid name"),
    }
}

#[tokio::test]
async fn bridge_configuration_creates_bridge_and_returns_snapshot() {
    // ARRANGE
    let mock = MockNetlinkOps::new();
    let idx = mock.add_link("eth0", [0xAA; 6], true);
    let snapshot = make_configured_snapshot("eth0", idx, [0xAA; 6]);
    let handle = Actor::spawn(snapshot, mock.clone(), make_config());

    let (reply_tx, reply_rx) = oneshot::channel();

    // ACT
    handle
        .cmd_tx
        .send(Command::ConfigureBridge {
            bridge_name: "br0".to_owned(),
            stp: false,
            reply: reply_tx,
        })
        .await
        .expect("send failed");

    let bridge_snap = reply_rx
        .await
        .expect("reply dropped")
        .expect("bridge failed");

    // ASSERT
    assert_eq!(bridge_snap.name, "br0");
    assert_eq!(bridge_snap.state, Lifecycle::Discovered);
    assert_eq!(bridge_snap.link, State::Up);
    assert_eq!(bridge_snap.l3_owner, bridge_snap.name);
    assert_eq!(bridge_snap.mac, [0xAA; 6], "bridge inherits the port MAC");
    assert!(bridge_snap.ip.is_none(), "bridge starts without an IP");
    assert!(bridge_snap.lease.is_none(), "bridge starts without a lease");
    assert!(
        bridge_snap.dhcp_state.is_none(),
        "bridge starts without DHCP state"
    );
    assert!(bridge_snap.ipv6.is_none(), "bridge starts without IPv6");
}

#[tokio::test]
async fn bridge_port_drops_l3_and_points_to_owner() {
    // ARRANGE
    let mock = MockNetlinkOps::new();
    let idx = mock.add_link("eth1", [0xBB; 6], true);
    let snapshot = make_configured_snapshot("eth1", idx, [0xBB; 6]);
    let handle = Actor::spawn(snapshot, mock.clone(), make_config());

    let (reply_tx, reply_rx) = oneshot::channel();

    // ACT
    handle
        .cmd_tx
        .send(Command::ConfigureBridge {
            bridge_name: "br0".to_owned(),
            stp: true,
            reply: reply_tx,
        })
        .await
        .expect("send failed");

    let _bridge_snap = reply_rx
        .await
        .expect("reply dropped")
        .expect("bridge failed");

    // ASSERT
    let port_snap = handle.state_rx.borrow().clone();
    assert_eq!(port_snap.state, Lifecycle::Configured);
    assert!(port_snap.ip.is_none(), "port should drop its IP");
    assert!(port_snap.lease.is_none(), "port should drop its lease");
    assert!(
        port_snap.dhcp_state.is_none(),
        "port should drop its DHCP state"
    );
    assert_eq!(
        port_snap.l3_owner,
        Name::new("br0").expect("valid name"),
        "port should point to the bridge as its L3 owner"
    );
}

#[tokio::test]
async fn bridge_preserves_mac_from_port() {
    // ARRANGE
    let mock = MockNetlinkOps::new();
    let mac = [0xDE, 0xAD, 0xBE, 0xEF, 0x00, 0x01];
    let idx = mock.add_link("eth3", mac, true);
    let snapshot = make_configured_snapshot("eth3", idx, mac);
    let handle = Actor::spawn(snapshot, mock.clone(), make_config());

    let (reply_tx, reply_rx) = oneshot::channel();

    // ACT
    handle
        .cmd_tx
        .send(Command::ConfigureBridge {
            bridge_name: "br3".to_owned(),
            stp: false,
            reply: reply_tx,
        })
        .await
        .expect("send failed");

    let bridge_snap = reply_rx
        .await
        .expect("reply dropped")
        .expect("bridge failed");

    // ASSERT
    assert_eq!(bridge_snap.mac, mac, "bridge should inherit port's MAC");
}

#[tokio::test]
async fn bridge_with_stp_enabled() {
    // ARRANGE
    let mock = MockNetlinkOps::new();
    let idx = mock.add_link("eth4", [0xEE; 6], true);
    let snapshot = make_configured_snapshot("eth4", idx, [0xEE; 6]);
    let handle = Actor::spawn(snapshot, mock.clone(), make_config());

    let (reply_tx, reply_rx) = oneshot::channel();

    // ACT
    handle
        .cmd_tx
        .send(Command::ConfigureBridge {
            bridge_name: "br4".to_owned(),
            stp: true,
            reply: reply_tx,
        })
        .await
        .expect("send failed");

    let bridge_snap = reply_rx
        .await
        .expect("reply dropped")
        .expect("bridge failed");

    // ASSERT
    assert_eq!(bridge_snap.name, "br4");
    assert_eq!(bridge_snap.state, Lifecycle::Discovered);
}

#[tokio::test]
async fn bridge_without_carrier_reports_no_carrier() {
    // ARRANGE
    let mock = MockNetlinkOps::new();
    let idx = mock.add_link("eth5", [0xAA; 6], false);
    let snapshot = make_configured_snapshot("eth5", idx, [0xAA; 6]);
    let handle = Actor::spawn(snapshot, mock.clone(), make_config());

    let (reply_tx, reply_rx) = oneshot::channel();

    // ACT
    handle
        .cmd_tx
        .send(Command::ConfigureBridge {
            bridge_name: "br5".to_owned(),
            stp: false,
            reply: reply_tx,
        })
        .await
        .expect("send failed");

    let bridge_snap = reply_rx
        .await
        .expect("reply dropped")
        .expect("bridge failed");

    // ASSERT
    assert_eq!(
        bridge_snap.link,
        State::NoCarrier,
        "a bridge over a carrier-less port must not report up"
    );
}

#[tokio::test]
async fn bridge_with_invalid_name_returns_error() {
    // ARRANGE
    let mock = MockNetlinkOps::new();
    let idx = mock.add_link("eth6", [0x11; 6], true);
    let snapshot = make_configured_snapshot("eth6", idx, [0x11; 6]);
    let handle = Actor::spawn(snapshot, mock.clone(), make_config());

    let (reply_tx, reply_rx) = oneshot::channel();

    // ACT
    handle
        .cmd_tx
        .send(Command::ConfigureBridge {
            bridge_name: "a".repeat(16),
            stp: false,
            reply: reply_tx,
        })
        .await
        .expect("send failed");

    let result = reply_rx.await.expect("reply dropped");

    // ASSERT
    assert!(result.is_err(), "should fail with an oversized bridge name");
}
