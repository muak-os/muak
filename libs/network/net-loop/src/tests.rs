//! Effect dispatch tests that need the crate-private helpers.

use alloc::collections::VecDeque;
use core::future::Future;
use core::pin::pin;
use core::task::{Context, Poll, Waker};
use std::time::{Instant, SystemTime};

use net_runtime::mock::Mock;

use crate::{Loop, Pending, Progress, SocketOwner, SocketTable};

fn block_on<F: Future>(future: F) -> F::Output {
    let mut future = pin!(future);
    let waker = Waker::noop();
    let mut context = Context::from_waker(waker);
    match Future::poll(future.as_mut(), &mut context) {
        Poll::Ready(value) => value,
        Poll::Pending => panic!("mock future must be ready"),
    }
}

fn name(text: &str) -> netmodel::Name {
    netmodel::Name::new(text).expect("valid name")
}

fn link(text: &str, index: u32) -> ip_method::LinkInfo {
    ip_method::LinkInfo {
        name: name(text),
        index,
        mac: netmodel::Mac::default(),
        state: netmodel::LinkState::Up,
    }
}

fn owner(interface: &str, slot: ip_method::SocketSlot) -> SocketOwner {
    SocketOwner {
        interface: name(interface),
        family: iface::Family::V4,
        slot,
        kind: ip_method::SocketKind::RawL2,
    }
}

fn spec(kind: ip_method::SocketKind) -> ip_method::SocketSpec {
    ip_method::SocketSpec {
        slot: ip_method::SocketSlot::Primary,
        kind,
        link: name("eth0"),
        bind: None,
        peer: None,
    }
}

async fn apply_interface(
    mock: &Mock,
    interface: &netmodel::Name,
    sockets: &mut SocketTable,
    effect: iface::Effect<'_>,
) -> std::io::Result<()> {
    let mut pending = VecDeque::new();
    crate::interface_effects::apply(mock, interface, sockets, &mut pending, effect).await
}

fn runtime() -> Mock {
    let mock = Mock::new(Instant::now(), SystemTime::UNIX_EPOCH);
    mock.set_link(netmodel::Link {
        name: name("eth0"),
        index: 2,
        mac: netmodel::Mac::default(),
        state: netmodel::LinkState::Up,
        is_virtual: false,
        master: None,
    });
    mock
}

#[test]
fn open_send_close_dispatch_on_the_socket_kind() {
    // ARRANGE
    let mock = runtime();
    let interface = name("eth0");
    let mut sockets = SocketTable::new();
    let open = iface::Effect::OpenSocket {
        family: iface::Family::V4,
        spec: spec(ip_method::SocketKind::RawL2),
    };

    // ACT
    block_on(apply_interface(&mock, &interface, &mut sockets, open)).expect("open socket");
    let id = sockets
        .find(
            &interface,
            iface::Family::V4,
            ip_method::SocketSlot::Primary,
        )
        .expect("socket is tracked");
    block_on(apply_interface(
        &mock,
        &interface,
        &mut sockets,
        iface::Effect::Send {
            family: iface::Family::V4,
            slot: ip_method::SocketSlot::Primary,
            frame: &[7, 8],
            dst: None,
        },
    ))
    .expect("send frame");
    let sent = mock.sent(id).expect("sent frames");
    block_on(apply_interface(
        &mock,
        &interface,
        &mut sockets,
        iface::Effect::CloseSocket {
            family: iface::Family::V4,
            slot: ip_method::SocketSlot::Primary,
        },
    ))
    .expect("close socket");

    // ASSERT
    assert_eq!(sent, vec![vec![7, 8]]);
    assert!(sockets.is_empty());
    assert_eq!(
        mock.sent(id).expect_err("closed socket").kind(),
        std::io::ErrorKind::NotFound
    );
}

#[test]
fn send_without_a_tracked_socket_fails() {
    // ARRANGE
    let mock = runtime();
    let mut sockets = SocketTable::new();

    // ACT
    let error = block_on(apply_interface(
        &mock,
        &name("eth0"),
        &mut sockets,
        iface::Effect::Send {
            family: iface::Family::V6,
            slot: ip_method::SocketSlot::Secondary,
            frame: &[1],
            dst: None,
        },
    ))
    .expect_err("no socket is open");

    // ASSERT
    assert_eq!(error.kind(), std::io::ErrorKind::NotFound);
}

#[test]
fn icmpv6_send_uses_the_destination_from_the_effect() {
    // ARRANGE
    let mock = runtime();
    let interface = name("eth0");
    let mut sockets = SocketTable::new();
    let address = core::net::Ipv6Addr::LOCALHOST;
    let dst = core::net::SocketAddr::V6(core::net::SocketAddrV6::new(address, 0, 0, 0));

    // ACT
    block_on(apply_interface(
        &mock,
        &interface,
        &mut sockets,
        iface::Effect::OpenSocket {
            family: iface::Family::V6,
            spec: spec(ip_method::SocketKind::RawIcmpv6),
        },
    ))
    .expect("open socket");
    let id = sockets
        .find(
            &interface,
            iface::Family::V6,
            ip_method::SocketSlot::Primary,
        )
        .expect("socket is tracked");
    block_on(apply_interface(
        &mock,
        &interface,
        &mut sockets,
        iface::Effect::Send {
            family: iface::Family::V6,
            slot: ip_method::SocketSlot::Primary,
            frame: &[3, 4],
            dst: Some(dst),
        },
    ))
    .expect("send frame");

    // ASSERT
    let sent = mock.sent_with_peer(id).expect("sent frames");
    assert_eq!(sent, vec![(vec![3, 4], Some(address))]);
}

#[test]
fn icmpv6_send_without_a_destination_is_invalid() {
    // ARRANGE
    let mock = runtime();
    let interface = name("eth0");
    let mut sockets = SocketTable::new();
    block_on(apply_interface(
        &mock,
        &interface,
        &mut sockets,
        iface::Effect::OpenSocket {
            family: iface::Family::V6,
            spec: spec(ip_method::SocketKind::RawIcmpv6),
        },
    ))
    .expect("open socket");

    // ACT
    let error = block_on(apply_interface(
        &mock,
        &interface,
        &mut sockets,
        iface::Effect::Send {
            family: iface::Family::V6,
            slot: ip_method::SocketSlot::Primary,
            frame: &[3, 4],
            dst: None,
        },
    ))
    .expect_err("destination is required");

    // ASSERT
    assert_eq!(error.kind(), std::io::ErrorKind::InvalidInput);
}

#[test]
fn discover_queues_the_link_inventory() {
    // ARRANGE
    let mock = runtime();
    let mut pending = VecDeque::new();

    // ACT
    block_on(crate::supervisor_effects::apply(
        &mock,
        &mut pending,
        net_supervisor::Effect::Discover,
    ))
    .expect("discover links");

    // ASSERT
    let Some(Pending::Links(links)) = pending.pop_front() else {
        panic!("link inventory expected");
    };
    assert_eq!(links.len(), 1);
    assert_eq!(links[0].index, 2);
}

#[test]
fn probe_carriers_queues_the_state_per_link() {
    // ARRANGE
    let mock = runtime();
    let mut pending = VecDeque::new();
    let links = [
        netmodel::Link {
            name: name("eth0"),
            index: 2,
            mac: netmodel::Mac::default(),
            state: netmodel::LinkState::Up,
            is_virtual: false,
            master: None,
        },
        netmodel::Link {
            name: name("eth1"),
            index: 3,
            mac: netmodel::Mac::default(),
            state: netmodel::LinkState::NoCarrier,
            is_virtual: false,
            master: None,
        },
    ];

    // ACT
    block_on(crate::supervisor_effects::apply(
        &mock,
        &mut pending,
        net_supervisor::Effect::ProbeCarriers { links: &links },
    ))
    .expect("probe carriers");

    // ASSERT
    let Some(Pending::Carriers(carriers)) = pending.pop_front() else {
        panic!("carrier state expected");
    };
    assert_eq!(carriers, vec![(2, true), (3, false)]);
}

#[test]
fn ensure_bridge_reaches_the_runtime() {
    // ARRANGE
    let mock = runtime();
    let mut pending = VecDeque::new();
    let config = net_supervisor::BridgeConfig {
        port: Vec::new(),
        stp: true,
    };

    // ACT
    block_on(crate::supervisor_effects::apply(
        &mock,
        &mut pending,
        net_supervisor::Effect::EnsureBridge {
            name: name("br0"),
            config: &config,
        },
    ))
    .expect("ensure bridge");

    // ASSERT
    assert_eq!(mock.bridge_stp(&name("br0")), Some(true));
    assert!(pending.is_empty());
}

#[test]
fn configure_interface_is_cloned_for_the_next_step() {
    // ARRANGE
    let mock = runtime();
    let mut pending = VecDeque::new();
    let v4 = netmodel::Ipv4Config::default();

    // ACT
    block_on(crate::supervisor_effects::apply(
        &mock,
        &mut pending,
        net_supervisor::Effect::ConfigureInterface {
            name: name("eth0"),
            v4: Some(&v4),
            v6: None,
        },
    ))
    .expect("configure interface");

    // ASSERT
    let Some(Pending::ConfigureInterface {
        name: configured,
        v4,
        v6,
    }) = pending.pop_front()
    else {
        panic!("configure command expected");
    };
    assert_eq!(configured, name("eth0"));
    assert!(v4.is_some());
    assert!(v6.is_none());
}

#[test]
fn step_consumes_one_pending_input_and_works() {
    // ARRANGE
    let mut driver: Loop<Mock> = Loop::new(runtime(), net_supervisor::Config::default());
    driver.add_interface(link("eth0", 2));
    driver
        .pending
        .push_back(Pending::ShutdownInterface { name: name("eth0") });
    let now = Instant::now();

    // ACT
    let first = block_on(driver.step(now, SystemTime::UNIX_EPOCH)).expect("first step");
    let second = block_on(driver.step(now, SystemTime::UNIX_EPOCH)).expect("second step");

    // ASSERT
    assert_eq!(first, Progress::Worked);
    assert_eq!(second, Progress::Idle);
    assert!(driver.pending.is_empty());
}

#[test]
fn new_loop_is_idle_and_empty() {
    // ARRANGE
    let now = Instant::now();
    let mut driver: Loop<Mock> = Loop::new(runtime(), net_supervisor::Config::default());

    // ACT
    let progress = block_on(driver.step(now, SystemTime::UNIX_EPOCH)).expect("step");

    // ASSERT
    assert_eq!(progress, Progress::Idle);
    assert!(driver.interfaces().is_empty());
    assert!(driver.sockets().is_empty());
}

#[test]
fn add_interface_exposes_the_controller_without_a_wake() {
    // ARRANGE
    let now = Instant::now();
    let mut driver: Loop<Mock> = Loop::new(runtime(), net_supervisor::Config::default());

    // ACT
    driver.add_interface(link("eth1", 3));
    let controllers = driver.interfaces();

    // ASSERT
    assert_eq!(controllers.len(), 1);
    assert_eq!(controllers[0].link().index, 3);
    assert!(driver.next_wake(now).is_none());
}

#[test]
fn table_tracks_insert_get_and_remove() {
    // ARRANGE
    let mut table = SocketTable::new();
    let id = net_runtime::SocketId::new(7);

    // ACT
    let replaced = table.insert(id, owner("eth0", ip_method::SocketSlot::Primary));
    let stored = table.get(id).cloned();
    let removed = table.remove(id);

    // ASSERT
    assert!(replaced.is_none());
    assert_eq!(stored, Some(owner("eth0", ip_method::SocketSlot::Primary)));
    assert_eq!(removed, Some(owner("eth0", ip_method::SocketSlot::Primary)));
    assert!(table.get(id).is_none());
}

#[test]
fn table_replaces_the_owner_of_a_known_handle() {
    // ARRANGE
    let mut table = SocketTable::new();
    let id = net_runtime::SocketId::new(1);
    table.insert(id, owner("eth0", ip_method::SocketSlot::Primary));

    // ACT
    let replaced = table.insert(id, owner("eth1", ip_method::SocketSlot::Secondary));

    // ASSERT
    assert_eq!(
        replaced,
        Some(owner("eth0", ip_method::SocketSlot::Primary))
    );
    assert_eq!(
        table.get(id),
        Some(&owner("eth1", ip_method::SocketSlot::Secondary))
    );
    assert_eq!(table.len(), 1);
}

#[test]
fn table_finds_a_socket_by_owner() {
    // ARRANGE
    let mut table = SocketTable::new();
    let primary = net_runtime::SocketId::new(1);
    let secondary = net_runtime::SocketId::new(2);
    table.insert(primary, owner("eth0", ip_method::SocketSlot::Primary));
    table.insert(secondary, owner("eth1", ip_method::SocketSlot::Secondary));

    // ACT
    let found_primary = table.find(
        &name("eth0"),
        iface::Family::V4,
        ip_method::SocketSlot::Primary,
    );
    let found_secondary = table.find(
        &name("eth1"),
        iface::Family::V4,
        ip_method::SocketSlot::Secondary,
    );
    let missing = table.find(
        &name("eth0"),
        iface::Family::V6,
        ip_method::SocketSlot::Primary,
    );

    // ASSERT
    assert_eq!(found_primary, Some(primary));
    assert_eq!(found_secondary, Some(secondary));
    assert!(missing.is_none());
}

#[test]
fn table_len_and_iteration_follow_the_entries() {
    // ARRANGE
    let mut table = SocketTable::new();
    let empty = table.is_empty();

    // ACT
    table.insert(
        net_runtime::SocketId::new(1),
        owner("eth0", ip_method::SocketSlot::Primary),
    );
    table.insert(
        net_runtime::SocketId::new(2),
        owner("eth0", ip_method::SocketSlot::Secondary),
    );
    let open = table.len();
    let ids: Vec<u32> = table.iter().map(|entry| entry.id.get()).collect();
    table.remove(net_runtime::SocketId::new(1));

    // ASSERT
    assert!(empty);
    assert_eq!(open, 2);
    assert_eq!(ids, vec![1, 2]);
    assert!(!table.is_empty());
    assert_eq!(table.len(), 1);
}

#[test]
fn address_add_and_remove_carry_the_prefix_length() {
    // ARRANGE
    let mock = runtime();
    let interface = name("eth0");
    let mut sockets = SocketTable::new();
    let address = core::net::IpAddr::V4(core::net::Ipv4Addr::new(10, 0, 0, 2));

    // ACT
    block_on(apply_interface(
        &mock,
        &interface,
        &mut sockets,
        iface::Effect::AddAddress {
            link: name("eth0"),
            address,
            prefix_len: 24,
        },
    ))
    .expect("add address");
    let added = block_on(net_runtime::Address::list(&mock, &interface)).expect("list addresses");
    block_on(apply_interface(
        &mock,
        &interface,
        &mut sockets,
        iface::Effect::RemoveAddress {
            link: name("eth0"),
            address,
            prefix_len: 24,
        },
    ))
    .expect("remove address");
    let removed = block_on(net_runtime::Address::list(&mock, &interface)).expect("list addresses");

    // ASSERT
    assert_eq!(
        added,
        vec![netmodel::Address {
            address,
            prefix_len: 24,
        }]
    );
    assert_eq!(removed, Vec::<netmodel::Address>::new());
}

#[test]
fn set_link_up_and_down_reach_the_runtime() {
    // ARRANGE
    let mock = runtime();
    let interface = name("eth0");
    let mut sockets = SocketTable::new();

    // ACT
    block_on(apply_interface(
        &mock,
        &interface,
        &mut sockets,
        iface::Effect::SetLinkDown { link: name("eth0") },
    ))
    .expect("set link down");
    let down = block_on(net_runtime::Link::link(&mock, &interface))
        .expect("query link")
        .expect("known link");
    block_on(apply_interface(
        &mock,
        &interface,
        &mut sockets,
        iface::Effect::SetLinkUp { link: name("eth0") },
    ))
    .expect("set link up");
    let up = block_on(net_runtime::Link::link(&mock, &interface))
        .expect("query link")
        .expect("known link");

    // ASSERT
    assert_eq!(down.state, netmodel::LinkState::Down);
    assert_eq!(up.state, netmodel::LinkState::Up);
}

#[test]
fn ensure_and_remove_route_reach_the_runtime() {
    // ARRANGE
    let mock = runtime();
    let interface = name("eth0");
    let mut sockets = SocketTable::new();
    let gateway = core::net::IpAddr::V4(core::net::Ipv4Addr::new(10, 0, 0, 1));

    // ACT
    block_on(apply_interface(
        &mock,
        &interface,
        &mut sockets,
        iface::Effect::EnsureRoute {
            link: name("eth0"),
            gateway,
        },
    ))
    .expect("ensure route");
    let installed = mock.default_routes();
    block_on(apply_interface(
        &mock,
        &interface,
        &mut sockets,
        iface::Effect::RemoveRoute {
            link: name("eth0"),
            gateway,
        },
    ))
    .expect("remove route");
    let removed = mock.default_routes();

    // ASSERT
    assert_eq!(installed, vec![(name("eth0"), gateway)]);
    assert_eq!(removed, Vec::<(netmodel::Name, core::net::IpAddr)>::new());
}

#[test]
fn publish_effects_are_no_ops() {
    // ARRANGE
    let mock = runtime();
    let interface = name("eth0");
    let mut sockets = SocketTable::new();
    let mut pending = VecDeque::new();

    // ACT
    block_on(apply_interface(
        &mock,
        &interface,
        &mut sockets,
        iface::Effect::Publish,
    ))
    .expect("publish an interface");
    block_on(crate::supervisor_effects::apply(
        &mock,
        &mut pending,
        net_supervisor::Effect::Publish,
    ))
    .expect("publish the supervisor status");

    // ASSERT
    assert!(sockets.is_empty());
    assert!(pending.is_empty());
}

#[test]
fn open_rejects_a_spec_for_another_interface() {
    // ARRANGE
    let mock = runtime();
    let interface = name("eth1");
    let mut sockets = SocketTable::new();

    // ACT
    let error = block_on(apply_interface(
        &mock,
        &interface,
        &mut sockets,
        iface::Effect::OpenSocket {
            family: iface::Family::V4,
            spec: spec(ip_method::SocketKind::RawL2),
        },
    ))
    .expect_err("spec link differs from the interface");

    // ASSERT
    assert_eq!(error.kind(), std::io::ErrorKind::InvalidInput);
    assert!(sockets.is_empty());
}

#[test]
fn reopened_slot_closes_the_stale_socket() {
    // ARRANGE
    let mock = runtime();
    let interface = name("eth0");
    let mut sockets = SocketTable::new();

    // ACT
    block_on(apply_interface(
        &mock,
        &interface,
        &mut sockets,
        iface::Effect::OpenSocket {
            family: iface::Family::V4,
            spec: spec(ip_method::SocketKind::RawL2),
        },
    ))
    .expect("first open");
    let stale = sockets
        .find(
            &interface,
            iface::Family::V4,
            ip_method::SocketSlot::Primary,
        )
        .expect("first socket is tracked");
    block_on(apply_interface(
        &mock,
        &interface,
        &mut sockets,
        iface::Effect::OpenSocket {
            family: iface::Family::V4,
            spec: spec(ip_method::SocketKind::RawL2),
        },
    ))
    .expect("second open");
    let current = sockets
        .find(
            &interface,
            iface::Family::V4,
            ip_method::SocketSlot::Primary,
        )
        .expect("second socket is tracked");

    // ASSERT
    assert_ne!(stale, current);
    assert_eq!(sockets.len(), 1);
    assert_eq!(
        mock.sent(stale).expect_err("stale socket is closed").kind(),
        std::io::ErrorKind::NotFound
    );
    mock.sent(current).expect("current socket is open");
}

#[test]
fn send_rejects_a_destination_on_non_icmpv6_sockets() {
    // ARRANGE
    let mock = runtime();
    let interface = name("eth0");
    let mut sockets = SocketTable::new();
    let dst = core::net::SocketAddr::V6(core::net::SocketAddrV6::new(
        core::net::Ipv6Addr::LOCALHOST,
        0,
        0,
        0,
    ));

    // ACT
    block_on(apply_interface(
        &mock,
        &interface,
        &mut sockets,
        iface::Effect::OpenSocket {
            family: iface::Family::V4,
            spec: spec(ip_method::SocketKind::RawL2),
        },
    ))
    .expect("open packet socket");
    let packet_error = block_on(apply_interface(
        &mock,
        &interface,
        &mut sockets,
        iface::Effect::Send {
            family: iface::Family::V4,
            slot: ip_method::SocketSlot::Primary,
            frame: &[1],
            dst: Some(dst),
        },
    ))
    .expect_err("packet sockets take no destination");
    block_on(apply_interface(
        &mock,
        &interface,
        &mut sockets,
        iface::Effect::OpenSocket {
            family: iface::Family::V4,
            spec: spec(ip_method::SocketKind::Udp),
        },
    ))
    .expect("open udp socket");
    let udp_error = block_on(apply_interface(
        &mock,
        &interface,
        &mut sockets,
        iface::Effect::Send {
            family: iface::Family::V4,
            slot: ip_method::SocketSlot::Primary,
            frame: &[1],
            dst: Some(dst),
        },
    ))
    .expect_err("datagram sockets take no destination");

    // ASSERT
    assert_eq!(packet_error.kind(), std::io::ErrorKind::InvalidInput);
    assert_eq!(udp_error.kind(), std::io::ErrorKind::InvalidInput);
}

#[test]
fn pending_input_forces_a_wake_at_now() {
    // ARRANGE
    let now = Instant::now();
    let mut driver: Loop<Mock> = Loop::new(runtime(), net_supervisor::Config::default());
    let idle = driver.next_wake(now);

    // ACT
    driver
        .pending
        .push_back(Pending::ShutdownInterface { name: name("eth0") });
    let queued = driver.next_wake(now);

    // ASSERT
    assert!(idle.is_none());
    assert_eq!(queued, Some(now));
}

#[test]
fn unknown_interface_command_reports_not_found() {
    // ARRANGE
    let now = Instant::now();
    let mut driver: Loop<Mock> = Loop::new(runtime(), net_supervisor::Config::default());
    driver
        .pending
        .push_back(Pending::ShutdownInterface { name: name("eth0") });

    // ACT
    let error = block_on(driver.step(now, SystemTime::UNIX_EPOCH)).expect_err("no controller");

    // ASSERT
    assert_eq!(error.kind(), std::io::ErrorKind::NotFound);
}

#[test]
fn configure_pending_input_reaches_the_interface() {
    // ARRANGE
    let now = Instant::now();
    let mut driver: Loop<Mock> = Loop::new(runtime(), net_supervisor::Config::default());
    driver.add_interface(link("eth0", 2));
    driver.pending.push_back(Pending::ConfigureInterface {
        name: name("eth0"),
        v4: None,
        v6: None,
    });

    // ACT
    let progress = block_on(driver.step(now, SystemTime::UNIX_EPOCH)).expect("step");

    // ASSERT
    assert_eq!(progress, Progress::Worked);
    assert!(driver.pending.is_empty());
}

#[test]
fn request_entropy_fills_and_queues_a_pending_input() {
    // ARRANGE
    let mock = runtime();
    let interface = name("eth0");
    let mut sockets = SocketTable::new();
    let mut pending = VecDeque::new();

    // ACT
    block_on(crate::interface_effects::apply(
        &mock,
        &interface,
        &mut sockets,
        &mut pending,
        iface::Effect::RequestEntropy {
            family: iface::Family::V4,
        },
    ))
    .expect("request entropy");

    // ASSERT
    let Some(Pending::Entropy {
        name: queued,
        family,
        value,
    }) = pending.pop_front()
    else {
        panic!("entropy input expected");
    };
    assert_eq!(queued, interface);
    assert_eq!(family, iface::Family::V4);
    assert_eq!(value, u32::from_le_bytes([0, 1, 2, 3]));
    assert!(pending.is_empty());
}

#[test]
fn driver_consumes_a_pending_entropy_input() {
    // ARRANGE
    let now = Instant::now();
    let mut driver: Loop<Mock> = Loop::new(runtime(), net_supervisor::Config::default());
    driver.add_interface(link("eth0", 2));
    driver.pending.push_back(Pending::Entropy {
        name: name("eth0"),
        family: iface::Family::V4,
        value: 0x0102_0304,
    });

    // ACT
    let progress = block_on(driver.step(now, SystemTime::UNIX_EPOCH)).expect("step");

    // ASSERT
    assert_eq!(progress, Progress::Worked);
    assert!(driver.pending.is_empty());
}
