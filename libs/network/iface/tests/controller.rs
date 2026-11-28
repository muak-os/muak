//! Controller tests over the sans-io method steps.

#![cfg(test)]

#[cfg(any(feature = "ipv4", feature = "ipv6"))]
use core::net::SocketAddr;
#[cfg(feature = "ipv6")]
use core::net::SocketAddrV6;
use core::net::{IpAddr, Ipv4Addr, Ipv6Addr};
#[cfg(feature = "ipv4")]
use std::time::Duration;
use std::time::{Instant, SystemTime};

use iface::{Command, Controller, Effect, Input, Step, V4View, V6View};
#[cfg(feature = "ipv4")]
use ip::v4::udp;
#[cfg(any(feature = "ipv4", feature = "ipv6"))]
use ip_method::SocketKind;
#[cfg(any(feature = "ipv4", feature = "ipv6"))]
use ip_method::SocketSlot;
use mac::Mac;
#[cfg(feature = "ipv6")]
use ndp::build::router::{Advertisement, advertisement as build_advertisement};
#[cfg(feature = "ipv6")]
use ndp::message::AdvertisementHeader;
#[cfg(feature = "ipv6")]
use ndp::types::PrefixInformation;
#[cfg(feature = "ipv6")]
use ndp::{ALL_ROUTERS, ROUTER_ADVERTISEMENT, ROUTER_SOLICITATION};
#[cfg(any(feature = "ipv4", feature = "ipv6"))]
use netmodel::Family;
use netmodel::{Ipv4Config, Ipv6Config, LinkState, Name};

const MAC: [u8; 6] = [0x00, 0x1a, 0x2b, 0x3c, 0x4d, 0x5e];
const ADDRESS_V4: Ipv4Addr = Ipv4Addr::new(10, 0, 0, 2);
const ADDRESS_V4_ALT: Ipv4Addr = Ipv4Addr::new(10, 0, 0, 3);
const GATEWAY_V4: Ipv4Addr = Ipv4Addr::new(10, 0, 0, 1);
const ADDRESS_V6: Ipv6Addr = Ipv6Addr::new(0xfd00, 0, 0, 0, 0, 0, 0, 2);
const GATEWAY_V6: Ipv6Addr = Ipv6Addr::new(0xfd00, 0, 0, 0, 0, 0, 0, 1);
#[cfg(feature = "ipv6")]
const ROUTER: Ipv6Addr = Ipv6Addr::new(0xfe80, 0, 0, 0, 0, 0, 0, 1);
#[cfg(feature = "ipv6")]
const PREFIX: Ipv6Addr = Ipv6Addr::new(0x2001, 0x0db8, 0, 0, 0, 0, 0, 0);
#[cfg(feature = "ipv6")]
const SLAAC_ADDRESS: Ipv6Addr = Ipv6Addr::new(0x2001, 0x0db8, 0, 0, 0x021a, 0x2bff, 0xfe3c, 0x4d5e);
#[cfg(feature = "ipv4")]
const DHCP_XID: u32 = 0x0BAD_F00D;
#[cfg(feature = "ipv4")]
const DHCP_SERVER: Ipv4Addr = Ipv4Addr::new(10, 0, 0, 1);
#[cfg(feature = "ipv4")]
const DHCP_ASSIGNED: Ipv4Addr = Ipv4Addr::new(10, 0, 0, 2);

enum Observed {
    AddAddress {
        link: Name,
        address: IpAddr,
        prefix_len: u8,
    },
    RemoveAddress,
    EnsureRoute {
        gateway: IpAddr,
    },
    RemoveRoute,
    #[cfg(any(feature = "ipv4", feature = "ipv6"))]
    Open {
        family: Family,
        spec: ip_method::SocketSpec,
    },
    #[cfg(any(feature = "ipv4", feature = "ipv6"))]
    Send {
        family: Family,
        slot: SocketSlot,
        dst: Option<SocketAddr>,
        first: Option<u8>,
    },
    #[cfg(any(feature = "ipv4", feature = "ipv6"))]
    Close {
        family: Family,
        slot: SocketSlot,
    },
    #[cfg(feature = "ipv4")]
    RequestEntropy {
        family: Family,
    },
    SetLinkUp,
    SetLinkDown,
    Publish,
}

fn link() -> ip_method::LinkInfo {
    ip_method::LinkInfo {
        name: Name::new("eth0").expect("valid name"),
        index: 2,
        mac: Mac::new(MAC),
        state: LinkState::Up,
    }
}

fn v4_config() -> Ipv4Config {
    Ipv4Config {
        addresses: vec![(ADDRESS_V4, 24)],
        gateway: Some(GATEWAY_V4),
        dns: Vec::new(),
        dhcp: false,
    }
}

fn v4_config_for(address: Ipv4Addr) -> Ipv4Config {
    Ipv4Config {
        addresses: vec![(address, 24)],
        gateway: Some(GATEWAY_V4),
        dns: Vec::new(),
        dhcp: false,
    }
}

fn v6_config() -> Ipv6Config {
    Ipv6Config {
        addresses: vec![(ADDRESS_V6, 64)],
        gateway: Some(GATEWAY_V6),
        dns: Vec::new(),
        autoconf: false,
    }
}

#[cfg(feature = "ipv4")]
fn dhcp_config() -> Ipv4Config {
    Ipv4Config {
        dhcp: true,
        ..Ipv4Config::default()
    }
}

#[cfg(feature = "ipv6")]
fn slaac_config() -> Ipv6Config {
    Ipv6Config {
        autoconf: true,
        ..Ipv6Config::default()
    }
}

fn observe(controller: &mut Controller, now: Instant, input: Input<'_>) -> Option<Observed> {
    let mut journal = net_observe::journal::Journal::new();

    observe_with(controller, &mut journal, now, input)
}

fn observe_with(
    controller: &mut Controller,
    journal: &mut net_observe::journal::Journal,
    now: Instant,
    input: Input<'_>,
) -> Option<Observed> {
    match controller.step(journal, now, SystemTime::UNIX_EPOCH, input) {
        Step::Wait => None,
        Step::Effect(effect) => Some(convert(effect)),
    }
}

fn poll(controller: &mut Controller, now: Instant) -> Option<Observed> {
    observe(controller, now, Input::Poll)
}

fn drain(controller: &mut Controller, now: Instant) -> Vec<Observed> {
    let mut journal = net_observe::journal::Journal::new();

    drain_with(controller, &mut journal, now)
}

fn drain_with(
    controller: &mut Controller,
    journal: &mut net_observe::journal::Journal,
    now: Instant,
) -> Vec<Observed> {
    let mut effects = Vec::new();
    while let Some(effect) = observe_with(controller, journal, now, Input::Poll) {
        effects.push(effect);
    }

    effects
}

fn convert(effect: Effect<'_>) -> Observed {
    match effect {
        Effect::AddAddress {
            link,
            address,
            prefix_len,
        } => Observed::AddAddress {
            link,
            address,
            prefix_len,
        },
        Effect::RemoveAddress { .. } => Observed::RemoveAddress,
        Effect::EnsureRoute { gateway, .. } => Observed::EnsureRoute { gateway },
        Effect::RemoveRoute { .. } => Observed::RemoveRoute,
        #[cfg(any(feature = "ipv4", feature = "ipv6"))]
        Effect::OpenSocket { family, spec } => Observed::Open { family, spec },
        #[cfg(any(feature = "ipv4", feature = "ipv6"))]
        Effect::Send {
            family,
            slot,
            frame,
            dst,
        } => Observed::Send {
            family,
            slot,
            dst,
            first: frame.first().copied(),
        },
        #[cfg(any(feature = "ipv4", feature = "ipv6"))]
        Effect::CloseSocket { family, slot } => Observed::Close { family, slot },
        #[cfg(feature = "ipv4")]
        Effect::RequestEntropy { family } => Observed::RequestEntropy { family },
        #[cfg(not(feature = "ipv4"))]
        Effect::RequestEntropy { .. } => Observed::Publish,
        Effect::SetLinkUp { .. } => Observed::SetLinkUp,
        Effect::SetLinkDown { .. } => Observed::SetLinkDown,
        Effect::Publish => Observed::Publish,
        #[cfg(not(any(feature = "ipv4", feature = "ipv6")))]
        _ => Observed::Publish,
    }
}

fn configure(
    controller: &mut Controller,
    now: Instant,
    v4: Option<Ipv4Config>,
    v6: Option<Ipv6Config>,
) -> Vec<Observed> {
    let mut journal = net_observe::journal::Journal::new();
    configure_with(controller, &mut journal, now, v4, v6)
}

fn configure_with(
    controller: &mut Controller,
    journal: &mut net_observe::journal::Journal,
    now: Instant,
    v4: Option<Ipv4Config>,
    v6: Option<Ipv6Config>,
) -> Vec<Observed> {
    let mut effects = Vec::new();
    if let Some(effect) = observe_with(
        controller,
        journal,
        now,
        Input::Command(Command::Configure { v4, v6 }),
    ) {
        effects.push(effect);
    }
    effects.extend(drain_with(controller, journal, now));

    effects
}

#[cfg(any(feature = "ipv4", feature = "ipv6"))]
fn method(
    controller: &mut Controller,
    now: Instant,
    family: Family,
    input: ip_method::Input<'_>,
) -> Option<Observed> {
    observe(controller, now, Input::Method { family, input })
}

#[cfg(feature = "ipv6")]
fn advertisement() -> Vec<u8> {
    let prefixes = [PrefixInformation {
        prefix: PREFIX,
        prefix_len: 64,
        on_link: true,
        autonomous: true,
        valid_lifetime: 7200,
        preferred_lifetime: 3600,
    }];
    let header = AdvertisementHeader {
        hop_limit: 64,
        managed_flag: false,
        other_flag: false,
        router_lifetime: 1800,
        reachable_time: 0,
        retrans_timer: 0,
    };
    let content = Advertisement {
        header,
        link_layer: None,
        mtu: None,
        prefixes: &prefixes,
        dns_lifetime: 0,
        dns_servers: &[],
    };
    let mut buffer = [0_u8; 256];
    let written =
        build_advertisement(ROUTER, ROUTER, &content, &mut buffer).expect("advertisement fits");

    written.to_vec()
}

#[cfg(feature = "ipv4")]
fn dhcp_frame(kind: u8) -> Vec<u8> {
    let mut buffer = [0_u8; 1500];
    let header =
        dhcp_proto::header::build_with(&mut buffer, DHCP_XID, MAC, Ipv4Addr::UNSPECIFIED, 0)
            .expect("DHCP header fits");
    let mut payload = header.to_vec();
    payload[16..20].copy_from_slice(&DHCP_ASSIGNED.octets());
    payload.extend_from_slice(&[
        53, 1, kind, 54, 4, 10, 0, 0, 1, 1, 4, 255, 255, 255, 0, 3, 4, 10, 0, 0, 1, 51, 4, 0, 0, 0,
        100, 255,
    ]);
    let source = ip::v4::Addr {
        ip: DHCP_SERVER,
        port: dhcp_proto::DHCP_SERVER_PORT,
    };
    let destination = ip::v4::Addr {
        ip: Ipv4Addr::BROADCAST,
        port: dhcp_proto::DHCP_CLIENT_PORT,
    };
    let mut frame = [0_u8; 1536];

    udp::wrap(&payload, source, destination, &mut frame)
        .expect("DHCP frame fits")
        .to_vec()
}

#[test]
fn new_snapshots_the_link_state() {
    // ARRANGE
    let link = link();

    // ACT
    let controller = Controller::new(link);

    // ASSERT
    let snapshot = controller.snapshot();
    assert_eq!(snapshot.link, LinkState::Up);
    assert!(snapshot.v4.is_none());
    assert!(snapshot.v6.is_none());
    assert_eq!(snapshot.notices.len(), 0);
    assert_eq!(snapshot.v4_state, netmodel::MethodState::Stopped);
    assert_eq!(snapshot.v6_state, netmodel::MethodState::Stopped);
}

#[test]
fn static_v4_configure_adds_address_route_and_binds() {
    // ARRANGE
    let now = Instant::now();
    let mut controller = Controller::new(link());

    // ACT
    let effects = configure(&mut controller, now, Some(v4_config()), None);

    // ASSERT
    assert!(effects.iter().any(|effect| matches!(
        effect,
        Observed::AddAddress { link, address, prefix_len }
            if link == "eth0" && *address == IpAddr::V4(ADDRESS_V4) && *prefix_len == 24
    )));
    assert!(effects.iter().any(|effect| matches!(
        effect,
        Observed::EnsureRoute { gateway } if *gateway == IpAddr::V4(GATEWAY_V4)
    )));
    assert_eq!(controller.snapshot().v4_state, netmodel::MethodState::Bound);
    let Some(V4View::Static(view)) = controller.snapshot().v4.clone() else {
        panic!("static view expected");
    };
    assert_eq!(view.address(), ADDRESS_V4);
    assert_eq!(view.prefix_len(), 24);
    assert_eq!(view.gateway(), Some(GATEWAY_V4));
    assert!(controller.next_wake(now).is_none());
}

#[cfg(feature = "ipv4")]
#[test]
fn static_v4_overflow_configures_no_method_and_warns() {
    // ARRANGE
    let now = Instant::now();
    let mut controller = Controller::new(link());
    let mut journal = net_observe::journal::Journal::new();
    let addresses = vec![(ADDRESS_V4, 24); ip_static::MAX_ADDRESSES + 1];

    // ACT
    let effects = configure_with(
        &mut controller,
        &mut journal,
        now,
        Some(Ipv4Config {
            addresses,
            gateway: Some(GATEWAY_V4),
            dns: Vec::new(),
            dhcp: true,
        }),
        None,
    );
    let notices: Vec<_> = journal
        .drain()
        .filter_map(|observation| {
            if let net_observe::Event::Notice { text, .. } = observation.event {
                Some(text)
            } else {
                None
            }
        })
        .collect();

    // ASSERT
    assert!(
        notices
            .iter()
            .any(|notice| notice.contains("ipv4 configuration exceeds"))
    );
    assert!(controller.snapshot().v4.is_none());
    assert!(
        !effects
            .iter()
            .any(|effect| matches!(effect, Observed::AddAddress { .. }))
    );
    assert!(
        !effects
            .iter()
            .any(|effect| matches!(effect, Observed::RequestEntropy { .. }))
    );
}

#[test]
fn static_v6_configure_adds_address_route_and_binds() {
    // ARRANGE
    let now = Instant::now();
    let mut controller = Controller::new(link());

    // ACT
    let effects = configure(&mut controller, now, None, Some(v6_config()));

    // ASSERT
    assert!(effects.iter().any(|effect| matches!(
        effect,
        Observed::AddAddress { link, address, prefix_len }
            if link == "eth0" && *address == IpAddr::V6(ADDRESS_V6) && *prefix_len == 64
    )));
    assert!(effects.iter().any(|effect| matches!(
        effect,
        Observed::EnsureRoute { gateway } if *gateway == IpAddr::V6(GATEWAY_V6)
    )));
    let Some(V6View::Static(view)) = controller.snapshot().v6.clone() else {
        panic!("static view expected");
    };
    assert_eq!(view.address(), ADDRESS_V6);
    assert_eq!(controller.snapshot().v6_state, netmodel::MethodState::Bound);
}

#[cfg(all(feature = "ipv4", feature = "ipv6"))]
#[test]
fn dual_family_methods_interleave_and_wake_at_minimum() {
    // ARRANGE
    let now = Instant::now();
    let mut controller = Controller::new(link());

    // ACT
    let configured = configure(
        &mut controller,
        now,
        Some(dhcp_config()),
        Some(slaac_config()),
    );
    drop(method(
        &mut controller,
        now,
        Family::V4,
        ip_method::Input::SocketOpened {
            slot: SocketSlot::Primary,
        },
    ));
    let v4_sent = observe(
        &mut controller,
        now,
        Input::Method {
            family: Family::V4,
            input: ip_method::Input::Entropy(DHCP_XID),
        },
    );
    drop(method(
        &mut controller,
        now,
        Family::V6,
        ip_method::Input::SocketOpened {
            slot: SocketSlot::Primary,
        },
    ));
    let v6_sent = poll(&mut controller, now);
    let wake = controller.next_wake(now);

    // ASSERT
    assert!(configured.iter().any(|effect| matches!(
        effect,
        Observed::Open {
            family: Family::V4,
            ..
        }
    )));
    assert!(configured.iter().any(|effect| matches!(
        effect,
        Observed::Open {
            family: Family::V6,
            ..
        }
    )));
    assert!(matches!(
        v4_sent,
        Some(Observed::Send {
            family: Family::V4,
            ..
        })
    ));
    assert!(matches!(
        v6_sent,
        Some(Observed::Send {
            family: Family::V6,
            ..
        })
    ));
    assert_eq!(wake, Some(now + Duration::from_secs(4)));
}

#[cfg(feature = "ipv4")]
#[test]
fn dhcp_configure_routes_socket_frame_and_entropy_inputs() {
    // ARRANGE
    let now = Instant::now();
    let mut controller = Controller::new(link());

    // ACT
    let configured = configure(&mut controller, now, Some(dhcp_config()), None);
    let requested = method(
        &mut controller,
        now,
        Family::V4,
        ip_method::Input::SocketOpened {
            slot: SocketSlot::Primary,
        },
    );
    let sent = observe(
        &mut controller,
        now,
        Input::Method {
            family: Family::V4,
            input: ip_method::Input::Entropy(DHCP_XID),
        },
    );

    // ASSERT
    assert!(configured.iter().any(|effect| matches!(
        effect,
        Observed::Open { family: Family::V4, spec }
            if spec.kind == SocketKind::RawL2 && spec.slot == SocketSlot::Primary
    )));
    assert!(matches!(
        requested,
        Some(Observed::RequestEntropy { family: Family::V4 })
    ));
    assert!(matches!(
        sent,
        Some(Observed::Send {
            family: Family::V4,
            slot: SocketSlot::Primary,
            dst: None,
            first: Some(0x45),
        })
    ));
    assert_eq!(
        controller.next_wake(now),
        Some(now + Duration::from_secs(10))
    );
}

#[cfg(feature = "ipv4")]
#[test]
fn dhcp_frame_binds_and_exposes_the_dhcp_view() {
    // ARRANGE
    let now = Instant::now();
    let mut controller = Controller::new(link());
    let offer = dhcp_frame(dhcp_proto::message_type::OFFER);
    let ack = dhcp_frame(dhcp_proto::message_type::ACK);
    drop(configure(&mut controller, now, Some(dhcp_config()), None));
    drop(method(
        &mut controller,
        now,
        Family::V4,
        ip_method::Input::SocketOpened {
            slot: SocketSlot::Primary,
        },
    ));
    drop(observe(
        &mut controller,
        now,
        Input::Method {
            family: Family::V4,
            input: ip_method::Input::Entropy(DHCP_XID),
        },
    ));

    // ACT
    let requested = observe(
        &mut controller,
        now,
        Input::Method {
            family: Family::V4,
            input: ip_method::Input::Frame {
                slot: SocketSlot::Primary,
                bytes: &offer,
                source: None,
            },
        },
    );
    drop(observe(
        &mut controller,
        now,
        Input::Method {
            family: Family::V4,
            input: ip_method::Input::Entropy(DHCP_XID),
        },
    ));
    drop(observe(
        &mut controller,
        now,
        Input::Method {
            family: Family::V4,
            input: ip_method::Input::Frame {
                slot: SocketSlot::Primary,
                bytes: &ack,
                source: None,
            },
        },
    ));
    let effects = drain(&mut controller, now);

    assert!(matches!(
        requested,
        Some(Observed::RequestEntropy { family: Family::V4 })
    ));

    // ASSERT
    assert!(
        effects
            .iter()
            .any(|effect| matches!(effect, Observed::EnsureRoute { .. }))
    );
    let Some(V4View::Dhcp(view)) = controller.snapshot().v4.clone() else {
        panic!("DHCP view expected");
    };
    assert_eq!(view.lease.assigned_ip, DHCP_ASSIGNED);
    assert_eq!(controller.snapshot().v4_state, netmodel::MethodState::Bound);
}

#[cfg(feature = "ipv6")]
#[test]
fn slaac_configure_routes_router_solicitation_and_advertisement() {
    // ARRANGE
    let now = Instant::now();
    let mut controller = Controller::new(link());
    let packet = advertisement();

    // ACT
    let configured = configure(&mut controller, now, None, Some(slaac_config()));
    let acquiring = method(
        &mut controller,
        now,
        Family::V6,
        ip_method::Input::SocketOpened {
            slot: SocketSlot::Primary,
        },
    );
    let solicited = poll(&mut controller, now);
    let added = observe(
        &mut controller,
        now,
        Input::Method {
            family: Family::V6,
            input: ip_method::Input::Frame {
                slot: SocketSlot::Primary,
                bytes: &packet,
                source: Some(IpAddr::V6(ROUTER)),
            },
        },
    );

    // ASSERT
    assert!(configured.iter().any(|effect| matches!(
        effect,
        Observed::Open { family: Family::V6, spec } if spec.kind == SocketKind::RawIcmpv6
    )));
    assert!(matches!(acquiring, Some(Observed::Publish)));
    assert!(matches!(
        solicited,
        Some(Observed::Send {
            family: Family::V6,
            slot: SocketSlot::Primary,
            dst: Some(dst),
            first: Some(ROUTER_SOLICITATION),
        }) if dst == SocketAddr::V6(SocketAddrV6::new(ALL_ROUTERS, 0, 0, 0))
    ));
    assert!(matches!(
        added,
        Some(Observed::AddAddress { address, prefix_len, .. })
            if address == IpAddr::V6(SLAAC_ADDRESS) && prefix_len == 64
    ));
}

#[cfg(feature = "ipv6")]
#[test]
fn slaac_advertisement_updates_the_snapshot() {
    // ARRANGE
    let now = Instant::now();
    let mut controller = Controller::new(link());
    let packet = advertisement();
    drop(configure(&mut controller, now, None, Some(slaac_config())));
    drop(method(
        &mut controller,
        now,
        Family::V6,
        ip_method::Input::SocketOpened {
            slot: SocketSlot::Primary,
        },
    ));
    drop(poll(&mut controller, now));

    // ACT
    drop(observe(
        &mut controller,
        now,
        Input::Method {
            family: Family::V6,
            input: ip_method::Input::Frame {
                slot: SocketSlot::Primary,
                bytes: &packet,
                source: Some(IpAddr::V6(ROUTER)),
            },
        },
    ));
    let effects = drain(&mut controller, now);

    // ASSERT
    assert!(
        effects
            .iter()
            .any(|effect| matches!(effect, Observed::EnsureRoute { .. }))
    );
    let Some(V6View::Slaac(view)) = controller.snapshot().v6.clone() else {
        panic!("slaac view expected");
    };
    assert_eq!(view.addresses, vec![(SLAAC_ADDRESS, 64)]);
    assert_eq!(view.routers, vec![ROUTER]);
    assert_eq!(controller.snapshot().v6_state, netmodel::MethodState::Bound);
}

#[cfg(feature = "ipv6")]
#[test]
fn slaac_shutdown_clears_the_view_before_closing_the_socket() {
    // ARRANGE
    let now = Instant::now();
    let mut controller = Controller::new(link());
    let packet = advertisement();
    drop(configure(&mut controller, now, None, Some(slaac_config())));
    drop(method(
        &mut controller,
        now,
        Family::V6,
        ip_method::Input::SocketOpened {
            slot: SocketSlot::Primary,
        },
    ));
    drop(poll(&mut controller, now));
    drop(observe(
        &mut controller,
        now,
        Input::Method {
            family: Family::V6,
            input: ip_method::Input::Frame {
                slot: SocketSlot::Primary,
                bytes: &packet,
                source: Some(IpAddr::V6(ROUTER)),
            },
        },
    ));
    drop(drain(&mut controller, now));

    // ACT
    let removed = observe(&mut controller, now, Input::Command(Command::Shutdown));
    let cleared = controller.snapshot().v6.clone();
    let closed = drain(&mut controller, now);

    // ASSERT
    assert!(matches!(removed, Some(Observed::RemoveAddress)));
    assert!(cleared.is_none());
    assert!(closed.iter().any(|effect| matches!(
        effect,
        Observed::Close {
            family: Family::V6,
            slot: SocketSlot::Primary
        }
    )));
    assert_eq!(
        controller.snapshot().v6_state,
        netmodel::MethodState::Stopped
    );
}

#[cfg(feature = "ipv6")]
#[test]
fn malformed_advertisement_records_a_notice() {
    // ARRANGE
    let now = Instant::now();
    let mut controller = Controller::new(link());
    drop(configure(&mut controller, now, None, Some(slaac_config())));
    drop(method(
        &mut controller,
        now,
        Family::V6,
        ip_method::Input::SocketOpened {
            slot: SocketSlot::Primary,
        },
    ));
    drop(poll(&mut controller, now));

    // ACT
    let published = observe(
        &mut controller,
        now,
        Input::Method {
            family: Family::V6,
            input: ip_method::Input::Frame {
                slot: SocketSlot::Primary,
                bytes: &[ROUTER_ADVERTISEMENT],
                source: Some(IpAddr::V6(ROUTER)),
            },
        },
    );

    // ASSERT
    assert!(matches!(published, Some(Observed::Publish)));
    assert_eq!(controller.snapshot().notices.len(), 1);
}

#[cfg(feature = "ipv6")]
#[test]
fn notices_are_capped_and_cleared_on_configure() {
    // ARRANGE
    let now = Instant::now();
    let mut controller = Controller::new(link());
    drop(configure(&mut controller, now, None, Some(slaac_config())));
    drop(method(
        &mut controller,
        now,
        Family::V6,
        ip_method::Input::SocketOpened {
            slot: SocketSlot::Primary,
        },
    ));
    drop(poll(&mut controller, now));

    // ACT
    for _ in 0_usize..12 {
        drop(observe(
            &mut controller,
            now,
            Input::Method {
                family: Family::V6,
                input: ip_method::Input::Frame {
                    slot: SocketSlot::Primary,
                    bytes: &[ROUTER_ADVERTISEMENT],
                    source: Some(IpAddr::V6(ROUTER)),
                },
            },
        ));
    }
    let capped = controller.snapshot().notices.len();
    drop(configure(&mut controller, now, Some(v4_config()), None));

    // ASSERT
    assert_eq!(capped, 8);
    assert_eq!(controller.snapshot().notices.len(), 0);
}

#[test]
fn link_down_and_up_flow_through_the_static_method() {
    // ARRANGE
    let now = Instant::now();
    let mut controller = Controller::new(link());
    drop(configure(&mut controller, now, Some(v4_config()), None));

    // ACT
    let down = observe(
        &mut controller,
        now,
        Input::Link(ip_method::LinkInfo {
            state: LinkState::Down,
            ..link()
        }),
    );
    let down_link = controller.snapshot().link;
    let down_state = controller.snapshot().v4_state;
    let up = observe(
        &mut controller,
        now,
        Input::Link(ip_method::LinkInfo {
            state: LinkState::Up,
            ..link()
        }),
    );

    // ASSERT
    assert!(matches!(down, Some(Observed::Publish)));
    assert_eq!(down_link, LinkState::Down);
    assert_eq!(down_state, netmodel::MethodState::Degraded);
    assert!(matches!(
        up,
        Some(Observed::AddAddress { address, .. }) if address == IpAddr::V4(ADDRESS_V4)
    ));
}

#[test]
fn link_update_without_methods_publishes_a_snapshot() {
    // ARRANGE
    let now = Instant::now();
    let mut controller = Controller::new(link());

    // ACT
    let published = observe(
        &mut controller,
        now,
        Input::Link(ip_method::LinkInfo {
            state: LinkState::Down,
            ..link()
        }),
    );

    // ASSERT
    assert!(matches!(published, Some(Observed::Publish)));
    assert_eq!(controller.snapshot().link, LinkState::Down);
    assert!(controller.next_wake(now).is_none());
}

#[test]
fn configure_without_methods_publishes_a_snapshot() {
    // ARRANGE
    let now = Instant::now();
    let mut controller = Controller::new(link());

    // ACT
    let published = observe(
        &mut controller,
        now,
        Input::Command(Command::Configure { v4: None, v6: None }),
    );

    // ASSERT
    assert!(matches!(published, Some(Observed::Publish)));
    assert_eq!(
        controller.snapshot().v4_state,
        netmodel::MethodState::Stopped
    );
    assert_eq!(
        controller.snapshot().v6_state,
        netmodel::MethodState::Stopped
    );
}

#[test]
fn reconcile_reapplies_the_static_configuration() {
    // ARRANGE
    let now = Instant::now();
    let mut controller = Controller::new(link());
    drop(configure(&mut controller, now, Some(v4_config()), None));

    // ACT
    let readded = observe(&mut controller, now, Input::Command(Command::Reconcile));

    // ASSERT
    assert!(matches!(
        readded,
        Some(Observed::AddAddress { address, .. }) if address == IpAddr::V4(ADDRESS_V4)
    ));
}

#[test]
fn reconfigure_stops_outgoing_static_before_installing_the_new_one() {
    // ARRANGE
    let now = Instant::now();
    let mut controller = Controller::new(link());
    drop(configure(&mut controller, now, Some(v4_config()), None));

    // ACT
    let removed = observe(
        &mut controller,
        now,
        Input::Command(Command::Configure {
            v4: Some(v4_config_for(ADDRESS_V4_ALT)),
            v6: None,
        }),
    );
    let unrouted = poll(&mut controller, now);
    let rest = drain(&mut controller, now);

    // ASSERT
    assert!(matches!(removed, Some(Observed::RemoveAddress)));
    assert!(matches!(unrouted, Some(Observed::RemoveRoute)));
    assert!(rest.iter().any(|effect| matches!(
        effect,
        Observed::AddAddress { address, .. } if *address == IpAddr::V4(ADDRESS_V4_ALT)
    )));
}

#[cfg(feature = "ipv4")]
#[test]
fn reconfigure_closes_outgoing_dhcp_socket_before_the_new_config() {
    // ARRANGE
    let now = Instant::now();
    let mut controller = Controller::new(link());
    drop(configure(&mut controller, now, Some(dhcp_config()), None));

    // ACT
    let closed = observe(
        &mut controller,
        now,
        Input::Command(Command::Configure {
            v4: Some(v4_config_for(ADDRESS_V4_ALT)),
            v6: None,
        }),
    );
    let rest = drain(&mut controller, now);

    // ASSERT
    assert!(matches!(
        closed,
        Some(Observed::Close {
            family: Family::V4,
            slot: SocketSlot::Primary
        })
    ));
    assert!(rest.iter().any(|effect| matches!(
        effect,
        Observed::AddAddress { address, .. } if *address == IpAddr::V4(ADDRESS_V4_ALT)
    )));
}

#[cfg(feature = "ipv4")]
#[test]
fn shutdown_closes_dhcp_sockets_and_stops_the_method() {
    // ARRANGE
    let now = Instant::now();
    let mut controller = Controller::new(link());
    drop(configure(&mut controller, now, Some(dhcp_config()), None));

    // ACT
    let closed = observe(&mut controller, now, Input::Command(Command::Shutdown));
    let stopped = drain(&mut controller, now);

    // ASSERT
    assert!(matches!(
        closed,
        Some(Observed::Close {
            family: Family::V4,
            slot: SocketSlot::Primary
        })
    ));
    assert!(
        stopped
            .iter()
            .any(|effect| matches!(effect, Observed::Publish))
    );
    let snapshot = controller.snapshot();
    assert_eq!(snapshot.v4_state, netmodel::MethodState::Stopped);
    assert!(snapshot.v4.is_none());
}

#[test]
fn v4_method_state_is_journaled() {
    // ARRANGE
    let now = Instant::now();
    let mut controller = Controller::new(link());
    let mut journal = net_observe::journal::Journal::new();

    // ACT
    drop(configure_with(
        &mut controller,
        &mut journal,
        now,
        Some(v4_config()),
        None,
    ));
    let states: Vec<_> = journal
        .drain()
        .filter_map(|observation| {
            if let net_observe::Event::InterfaceState { family, state, .. } = observation.event {
                Some((family, state))
            } else {
                None
            }
        })
        .collect();

    // ASSERT
    assert!(states.contains(&(Family::V4, netmodel::MethodState::Bound)));
}

#[test]
fn v6_method_state_is_journaled() {
    // ARRANGE
    let now = Instant::now();
    let mut controller = Controller::new(link());
    let mut journal = net_observe::journal::Journal::new();

    // ACT
    drop(configure_with(
        &mut controller,
        &mut journal,
        now,
        None,
        Some(v6_config()),
    ));
    let states: Vec<_> = journal
        .drain()
        .filter_map(|observation| {
            if let net_observe::Event::InterfaceState { family, state, .. } = observation.event {
                Some((family, state))
            } else {
                None
            }
        })
        .collect();

    // ASSERT
    assert!(states.contains(&(Family::V6, netmodel::MethodState::Bound)));
}

#[cfg(feature = "ipv4")]
#[test]
fn dhcp_lease_is_journaled_when_the_view_changes() {
    // ARRANGE
    let now = Instant::now();
    let mut controller = Controller::new(link());
    let mut journal = net_observe::journal::Journal::new();
    let offer = dhcp_frame(dhcp_proto::message_type::OFFER);
    let ack = dhcp_frame(dhcp_proto::message_type::ACK);

    // ACT
    drop(configure_with(
        &mut controller,
        &mut journal,
        now,
        Some(dhcp_config()),
        None,
    ));
    drop(observe_with(
        &mut controller,
        &mut journal,
        now,
        Input::Method {
            family: Family::V4,
            input: ip_method::Input::SocketOpened {
                slot: SocketSlot::Primary,
            },
        },
    ));
    drop(observe_with(
        &mut controller,
        &mut journal,
        now,
        Input::Method {
            family: Family::V4,
            input: ip_method::Input::Entropy(DHCP_XID),
        },
    ));

    drop(observe_with(
        &mut controller,
        &mut journal,
        now,
        Input::Method {
            family: Family::V4,
            input: ip_method::Input::Frame {
                slot: SocketSlot::Primary,
                bytes: &offer,
                source: None,
            },
        },
    ));
    drop(observe_with(
        &mut controller,
        &mut journal,
        now,
        Input::Method {
            family: Family::V4,
            input: ip_method::Input::Entropy(DHCP_XID),
        },
    ));
    drop(observe_with(
        &mut controller,
        &mut journal,
        now,
        Input::Method {
            family: Family::V4,
            input: ip_method::Input::Frame {
                slot: SocketSlot::Primary,
                bytes: &ack,
                source: None,
            },
        },
    ));
    drop(drain_with(&mut controller, &mut journal, now));
    let lease = journal.drain().find_map(|observation| {
        if let net_observe::Event::DhcpLease { lease, .. } = observation.event {
            Some(lease)
        } else {
            None
        }
    });

    // ASSERT
    let lease = lease.expect("DHCP lease observed");
    assert_eq!(lease.address, DHCP_ASSIGNED);
    assert_eq!(lease.prefix_len, 24);
    assert_eq!(lease.server, DHCP_SERVER);
    assert_eq!(lease.gateway, Some(DHCP_SERVER));
    assert_eq!(lease.dns, Vec::<Ipv4Addr>::new());
    assert_eq!(lease.phase, net_observe::interface::DhcpPhase::Bound);
}

#[cfg(feature = "ipv6")]
#[test]
fn slaac_update_is_journaled_when_the_view_changes() {
    // ARRANGE
    let now = Instant::now();
    let mut controller = Controller::new(link());
    let mut journal = net_observe::journal::Journal::new();
    let packet = advertisement();

    // ACT
    drop(configure_with(
        &mut controller,
        &mut journal,
        now,
        None,
        Some(slaac_config()),
    ));
    drop(observe_with(
        &mut controller,
        &mut journal,
        now,
        Input::Method {
            family: Family::V6,
            input: ip_method::Input::SocketOpened {
                slot: SocketSlot::Primary,
            },
        },
    ));
    drop(observe_with(
        &mut controller,
        &mut journal,
        now,
        Input::Poll,
    ));

    drop(observe_with(
        &mut controller,
        &mut journal,
        now,
        Input::Method {
            family: Family::V6,
            input: ip_method::Input::Frame {
                slot: SocketSlot::Primary,
                bytes: &packet,
                source: Some(IpAddr::V6(ROUTER)),
            },
        },
    ));
    let update = journal.drain().find_map(|observation| {
        if let net_observe::Event::SlaacUpdate { update, .. } = observation.event {
            Some(update)
        } else {
            None
        }
    });

    // ASSERT
    let update = update.expect("SLAAC update observed");
    assert_eq!(update.addresses, vec![(SLAAC_ADDRESS, 64)]);
    assert_eq!(update.routers, vec![ROUTER]);
}

#[cfg(not(feature = "ipv4"))]
#[test]
fn ipv4_replacement_notice_is_journaled() {
    // ARRANGE
    let now = Instant::now();
    let mut controller = Controller::new(link());
    let mut journal = net_observe::journal::Journal::new();

    // ACT
    drop(configure_with(
        &mut controller,
        &mut journal,
        now,
        Some(Ipv4Config {
            dhcp: true,
            ..Ipv4Config::default()
        }),
        None,
    ));
    let notices: Vec<_> = journal
        .drain()
        .filter_map(|observation| {
            if let net_observe::Event::Notice { family, text, .. } = observation.event {
                Some((family, text))
            } else {
                None
            }
        })
        .collect();

    // ASSERT
    assert!(
        notices
            .iter()
            .any(|notice| notice.0.is_none() && notice.1.contains("ipv4 feature is disabled"))
    );
}

#[cfg(not(feature = "ipv6"))]
#[test]
fn ipv6_replacement_notice_is_journaled() {
    // ARRANGE
    let now = Instant::now();
    let mut controller = Controller::new(link());
    let mut journal = net_observe::journal::Journal::new();

    // ACT
    drop(configure_with(
        &mut controller,
        &mut journal,
        now,
        None,
        Some(Ipv6Config {
            autoconf: true,
            ..Ipv6Config::default()
        }),
    ));
    let notices: Vec<_> = journal
        .drain()
        .filter_map(|observation| {
            if let net_observe::Event::Notice { family, text, .. } = observation.event {
                Some((family, text))
            } else {
                None
            }
        })
        .collect();

    // ASSERT
    assert!(
        notices
            .iter()
            .any(|notice| notice.0.is_none() && notice.1.contains("ipv6 feature is disabled"))
    );
}

#[cfg(all(feature = "ipv4", feature = "ipv6"))]
#[test]
fn replacement_with_enabled_features_journals_no_notice() {
    // ARRANGE
    let now = Instant::now();
    let mut controller = Controller::new(link());
    let mut journal = net_observe::journal::Journal::new();

    // ACT
    drop(configure_with(
        &mut controller,
        &mut journal,
        now,
        Some(Ipv4Config {
            dhcp: true,
            ..Ipv4Config::default()
        }),
        Some(Ipv6Config {
            autoconf: true,
            ..Ipv6Config::default()
        }),
    ));

    // ASSERT
    assert!(
        !journal
            .drain()
            .any(|observation| matches!(observation.event, net_observe::Event::Notice { .. }))
    );
}
