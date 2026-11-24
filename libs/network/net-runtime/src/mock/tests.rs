//! Behaviour tests for the deterministic mock runtime.

use core::future::Future;
use core::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use core::pin::pin;
use core::task::{Context, Poll, Waker};
use std::time::{Duration, Instant, SystemTime};

use super::Mock;
use crate::{Address, Clock as _, Entropy as _, Events, Icmpv6, Link, Packet, Route, Runtime, Udp};

fn runtime() -> Mock {
    Mock::new(Instant::now(), SystemTime::UNIX_EPOCH)
}

fn link(name: &netmodel::Name, index: u32) -> netmodel::Link {
    netmodel::Link {
        name: name.clone(),
        index,
        mac: netmodel::Mac::default(),
        state: netmodel::LinkState::Down,
        is_virtual: false,
        master: None,
    }
}

fn spec(kind: ip_method::SocketKind) -> ip_method::SocketSpec {
    ip_method::SocketSpec {
        slot: ip_method::SocketSlot::Primary,
        kind,
        link: netmodel::Name::new("eth0").expect("valid name"),
        bind: None,
        peer: None,
    }
}

fn block_on<F: Future>(future: F) -> F::Output {
    let mut future = pin!(future);
    let waker = Waker::noop();
    let mut context = Context::from_waker(waker);

    match Future::poll(future.as_mut(), &mut context) {
        Poll::Ready(value) => value,
        Poll::Pending => panic!("mock future must be ready"),
    }
}

fn bundle_clock<R: Runtime>(runtime: &R) -> Instant {
    runtime.clock().now()
}

fn assert_bundle<R: Runtime>() {}

#[test]
fn clock_is_settable() {
    // ARRANGE
    let start = Instant::now();
    let mock = Mock::new(start, SystemTime::UNIX_EPOCH);
    let later = start
        .checked_add(Duration::from_secs(5))
        .expect("no overflow");
    let wall = SystemTime::UNIX_EPOCH
        .checked_add(Duration::from_secs(60))
        .expect("no overflow");

    // ACT
    mock.set_now(later);
    mock.set_wall(wall);

    // ASSERT
    assert_eq!(mock.now(), later);
    assert_eq!(mock.wall_now(), wall);
}

#[test]
fn entropy_is_a_deterministic_counter() {
    // ARRANGE
    let first = runtime();
    let second = runtime();
    let mut left = [0_u8; 4];
    let mut right = [0_u8; 4];
    let mut next = [0_u8; 2];

    // ACT
    first.fill(&mut left);
    second.fill(&mut right);
    first.fill(&mut next);

    // ASSERT
    assert_eq!(left, [0, 1, 2, 3]);
    assert_eq!(right, left);
    assert_eq!(next, [4, 5]);
}

#[test]
fn packet_socket_round_trips_frames() {
    // ARRANGE
    let mock = runtime();
    let spec = spec(ip_method::SocketKind::RawL2);
    let mut buf = [0_u8; 8];

    // ACT
    let id = block_on(Packet::open(&mock, &spec)).expect("open socket");
    let sent = block_on(Packet::send(&mock, id, b"hello")).expect("send frame");
    mock.push_inbound(id, b"world").expect("queue frame");
    let received = block_on(Packet::recv(&mock, id, &mut buf)).expect("receive frame");
    let frames = mock.sent(id).expect("sent frames");
    let closed = block_on(Packet::close(&mock, id)).is_ok();
    let after_close = block_on(Packet::recv(&mock, id, &mut buf));

    // ASSERT
    assert_eq!(sent, 5);
    assert_eq!(received, 5);
    assert_eq!(&buf[..received], b"world");
    assert_eq!(frames, vec![b"hello".to_vec()]);
    assert!(closed);
    assert_eq!(
        after_close.expect_err("closed socket").kind(),
        std::io::ErrorKind::NotFound
    );
}

#[test]
fn icmpv6_socket_reports_source() {
    // ARRANGE
    let mock = runtime();
    let spec = spec(ip_method::SocketKind::RawIcmpv6);
    let source = Ipv6Addr::LOCALHOST;
    let mut buf = [0_u8; 4];

    // ACT
    let id = block_on(Icmpv6::open(&mock, &spec)).expect("open socket");
    let sent = block_on(Icmpv6::send(&mock, id, source, b"ping")).expect("send payload");
    mock.push_inbound_from(id, b"pong", source)
        .expect("queue frame");
    let (received, from) = block_on(Icmpv6::recv(&mock, id, &mut buf)).expect("receive payload");
    let with_peer = mock.sent_with_peer(id).expect("sent frames");

    // ASSERT
    assert_eq!(sent, 4);
    assert_eq!(received, 4);
    assert_eq!(from, Some(source));
    assert_eq!(&buf[..received], b"pong");
    assert_eq!(with_peer, vec![(b"ping".to_vec(), Some(source))]);
}

#[test]
fn udp_socket_round_trips_payloads_and_preserves_endpoints() {
    // ARRANGE
    let mock = runtime();
    let mut spec = spec(ip_method::SocketKind::Udp);
    let bind = SocketAddr::from((Ipv4Addr::UNSPECIFIED, 68));
    let peer = SocketAddr::from((Ipv4Addr::BROADCAST, 67));
    spec.bind = Some(bind);
    spec.peer = Some(peer);
    let mut buf = [0_u8; 8];

    // ACT
    let id = block_on(Udp::open(&mock, &spec)).expect("open socket");
    let sent = block_on(Udp::send(&mock, id, b"offer")).expect("send payload");
    mock.push_inbound_udp(id, b"ack").expect("queue payload");
    let received = block_on(Udp::recv(&mock, id, &mut buf)).expect("receive payload");
    let endpoints = mock.udp_endpoints(id).expect("endpoints");
    let payloads = mock.udp_sent(id).expect("sent payloads");
    let empty = block_on(Udp::recv(&mock, id, &mut buf));
    block_on(Udp::close(&mock, id)).expect("close socket");
    let after_close = block_on(Udp::recv(&mock, id, &mut buf));

    // ASSERT
    assert_eq!(sent, 5);
    assert_eq!(received, 3);
    assert_eq!(&buf[..received], b"ack");
    assert_eq!(endpoints, (Some(bind), Some(peer)));
    assert_eq!(payloads, vec![b"offer".to_vec()]);
    assert_eq!(
        empty.expect_err("empty socket").kind(),
        std::io::ErrorKind::WouldBlock
    );
    assert_eq!(
        after_close.expect_err("closed socket").kind(),
        std::io::ErrorKind::NotFound
    );
}

#[test]
fn udp_send_after_close_is_not_found() {
    // ARRANGE
    let mock = runtime();
    let spec = spec(ip_method::SocketKind::Udp);

    // ACT
    let id = block_on(Udp::open(&mock, &spec)).expect("open socket");
    block_on(Udp::close(&mock, id)).expect("close socket");
    let sent = block_on(Udp::send(&mock, id, b"nope"));

    // ASSERT
    assert_eq!(
        sent.expect_err("closed socket").kind(),
        std::io::ErrorKind::NotFound
    );
}

#[test]
fn udp_open_without_endpoints_reports_none() {
    // ARRANGE
    let mock = runtime();
    let spec = spec(ip_method::SocketKind::Udp);

    // ACT
    let id = block_on(Udp::open(&mock, &spec)).expect("open socket");
    let endpoints = mock.udp_endpoints(id).expect("endpoints");

    // ASSERT
    assert_eq!(endpoints, (None, None));
}

#[test]
fn socket_open_rejects_wrong_kind() {
    // ARRANGE
    let mock = runtime();
    let udp = spec(ip_method::SocketKind::Udp);
    let raw = spec(ip_method::SocketKind::RawL2);
    let icmp = spec(ip_method::SocketKind::RawIcmpv6);

    // ACT
    let packet = block_on(Packet::open(&mock, &udp));
    let icmpv6 = block_on(Icmpv6::open(&mock, &raw));
    let datagram = block_on(Udp::open(&mock, &icmp));

    // ASSERT
    for result in [packet, icmpv6, datagram] {
        assert_eq!(
            result.expect_err("wrong kind").kind(),
            std::io::ErrorKind::InvalidInput
        );
    }
}

#[test]
fn event_queue_delivers_in_order() {
    // ARRANGE
    let mock = runtime();
    let name = netmodel::Name::new("eth0").expect("valid name");
    let first = netmodel::LinkEvent::Up {
        name: name.clone(),
        index: 1,
    };
    let second = netmodel::LinkEvent::Deleted { name, index: 1 };
    mock.push_event(first.clone());
    mock.push_event(second.clone());

    // ACT
    let first_out = block_on(Events::next(&mock)).expect("first event");
    let second_out = block_on(Events::next(&mock)).expect("second event");
    let empty = block_on(Events::next(&mock));

    // ASSERT
    assert_eq!(first_out, first);
    assert_eq!(second_out, second);
    assert_eq!(
        empty.expect_err("empty queue").kind(),
        std::io::ErrorKind::WouldBlock
    );
}

#[test]
fn link_address_and_route_state_is_tracked() {
    // ARRANGE
    let mock = runtime();
    let name = netmodel::Name::new("eth0").expect("valid name");
    mock.set_link(link(&name, 3));
    let address = netmodel::Address {
        address: IpAddr::V4(Ipv4Addr::new(10, 0, 0, 2)),
        prefix_len: 24,
    };
    let gateway = IpAddr::V4(Ipv4Addr::new(10, 0, 0, 1));

    // ACT
    block_on(Link::set_up(&mock, &name)).expect("bring link up");
    block_on(Address::add(&mock, &name, address)).expect("add address");
    block_on(Route::ensure_default(&mock, &name, gateway)).expect("ensure route");
    let link = block_on(Link::link(&mock, &name)).expect("link query");
    let listed = block_on(Address::list(&mock, &name)).expect("address list");

    // ASSERT
    assert_eq!(link.expect("known link").state, netmodel::LinkState::Up);
    assert_eq!(listed, vec![address]);
    assert_eq!(mock.default_routes(), vec![(name, gateway)]);
}

#[test]
fn socket_id_round_trips_handle_value() {
    // ARRANGE
    let id = crate::SocketId::new(7);

    // ACT
    let value = id.get();

    // ASSERT
    assert_eq!(value, 7);
}

#[test]
fn link_master_and_detach_are_tracked() {
    // ARRANGE
    let mock = runtime();
    let master = netmodel::Name::new("br0").expect("valid name");
    let port = netmodel::Name::new("eth0").expect("valid name");
    mock.set_link(link(&master, 2));
    mock.set_link(link(&port, 3));

    // ACT
    block_on(Link::set_master(&mock, &port, &master)).expect("set master");
    let attached = block_on(Link::link(&mock, &port)).expect("link query");
    let all = block_on(Link::links(&mock)).expect("link list");
    block_on(Link::detach(&mock, &port)).expect("detach port");
    let detached = block_on(Link::link(&mock, &port)).expect("link query");

    // ASSERT
    assert_eq!(attached.expect("port").master, Some(2));
    assert_eq!(all.len(), 2);
    assert_eq!(detached.expect("port").master, None);
}

#[test]
fn ensure_bridge_records_stp() {
    // ARRANGE
    let mock = runtime();
    let bridge = netmodel::Name::new("br0").expect("valid name");

    // ACT
    block_on(Link::ensure_bridge(&mock, &bridge, true)).expect("create bridge");
    let enabled = mock.bridge_stp(&bridge);
    block_on(Link::ensure_bridge(&mock, &bridge, false)).expect("update bridge");
    let disabled = mock.bridge_stp(&bridge);
    let links = block_on(Link::links(&mock)).expect("link list");

    // ASSERT
    assert_eq!(enabled, Some(true));
    assert_eq!(disabled, Some(false));
    assert_eq!(links.len(), 1);
    assert!(links.first().expect("bridge").is_virtual);
}

#[test]
fn address_and_route_operations_require_known_link() {
    // ARRANGE
    let mock = runtime();
    let name = netmodel::Name::new("eth0").expect("valid name");
    let unknown = netmodel::Name::new("eth9").expect("valid name");
    mock.set_link(link(&name, 3));
    let address = netmodel::Address {
        address: IpAddr::V4(Ipv4Addr::new(10, 0, 0, 2)),
        prefix_len: 24,
    };
    let gateway = IpAddr::V4(Ipv4Addr::new(10, 0, 0, 1));

    // ACT
    block_on(Address::add(&mock, &name, address)).expect("add address");
    block_on(Address::remove(&mock, &name, address)).expect("remove address");
    block_on(Route::ensure_default(&mock, &name, gateway)).expect("ensure route");
    block_on(Route::remove_default(&mock, &name, gateway)).expect("remove route");
    let listed = block_on(Address::list(&mock, &name)).expect("address list");
    let missing_address = block_on(Address::add(&mock, &unknown, address));
    let missing_route = block_on(Route::ensure_default(&mock, &unknown, gateway));

    // ASSERT
    assert_eq!(listed, Vec::new());
    assert_eq!(mock.default_routes(), Vec::new());
    assert_eq!(
        missing_address.expect_err("unknown link").kind(),
        std::io::ErrorKind::NotFound
    );
    assert_eq!(
        missing_route.expect_err("unknown link").kind(),
        std::io::ErrorKind::NotFound
    );
}

#[test]
fn icmpv6_close_rejects_receive() {
    // ARRANGE
    let mock = runtime();
    let spec = spec(ip_method::SocketKind::RawIcmpv6);
    let mut buf = [0_u8; 4];

    // ACT
    let id = block_on(Icmpv6::open(&mock, &spec)).expect("open socket");
    block_on(Icmpv6::close(&mock, id)).expect("close socket");
    let received = block_on(Icmpv6::recv(&mock, id, &mut buf));
    let closed_again = block_on(Icmpv6::close(&mock, id));

    // ASSERT
    assert_eq!(
        received.expect_err("closed socket").kind(),
        std::io::ErrorKind::NotFound
    );
    assert_eq!(
        closed_again.expect_err("closed socket").kind(),
        std::io::ErrorKind::NotFound
    );
}

#[test]
fn runtime_bundle_exposes_ports() {
    // ARRANGE
    let start = Instant::now();
    let mock = Mock::new(start, SystemTime::UNIX_EPOCH);
    let name = netmodel::Name::new("eth0").expect("valid name");
    mock.set_link(link(&name, 2));
    let spec = spec(ip_method::SocketKind::RawL2);

    // ACT
    assert_bundle::<Mock>();
    let observed = bundle_clock(&mock);
    let links = block_on(Runtime::link(&mock).links()).expect("list links");
    let id = block_on(Packet::open(&mock, &spec)).expect("open socket");
    let sent = block_on(Packet::send(&mock, id, &[1, 2])).expect("send frame");

    // ASSERT
    assert_eq!(observed, start);
    assert_eq!(links, vec![link(&name, 2)]);
    assert_eq!(sent, 2);
    assert_eq!(mock.sent(id).expect("sent frames"), vec![vec![1, 2]]);
}
