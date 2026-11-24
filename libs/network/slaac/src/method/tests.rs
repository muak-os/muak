use core::net::{IpAddr, Ipv6Addr, SocketAddr, SocketAddrV6};
use std::io;
use std::time::{Duration, Instant, SystemTime};

use ip_method::{
    Command, Effect, Event, Failure, Input, Intent, LinkInfo, Method as _, SocketKind, SocketSlot,
    SocketSpec, State, Step,
};
use ndp::build::router::{Advertisement, advertisement as build_advertisement};
use ndp::message::AdvertisementHeader;
use ndp::parse::router_solicitation;
use ndp::types::PrefixInformation;
use ndp::{ALL_ROUTERS, NEIGHBOR_SOLICITATION, ROUTER_ADVERTISEMENT, ROUTER_SOLICITATION};
use netmodel::{Ipv6Config, LinkState, Mac, Name};

use crate::{Slaac, View};

const MAC: [u8; 6] = [0x00, 0x1a, 0x2b, 0x3c, 0x4d, 0x5e];
const ROUTER: Ipv6Addr = Ipv6Addr::new(0xfe80, 0, 0, 0, 0, 0, 0, 1);
const PREFIX: Ipv6Addr = Ipv6Addr::new(0x2001, 0x0db8, 0, 0, 0, 0, 0, 0);
const ADDRESS: Ipv6Addr = Ipv6Addr::new(0x2001, 0x0db8, 0, 0, 0x021a, 0x2bff, 0xfe3c, 0x4d5e);
const DNS: Ipv6Addr = Ipv6Addr::new(0x2001, 0x0db8, 0, 0, 0, 0, 0, 0x53);
const FALLBACK: Ipv6Addr = Ipv6Addr::new(0x2001, 0x0db8, 0, 0, 0, 0, 0, 0x54);
const ROUTER_B: Ipv6Addr = Ipv6Addr::new(0xfe80, 0, 0, 0, 0, 0, 0, 2);
const PREFIX_B: Ipv6Addr = Ipv6Addr::new(0x2001, 0x0db8, 1, 0, 0, 0, 0, 0);
const ADDRESS_B: Ipv6Addr = Ipv6Addr::new(0x2001, 0x0db8, 1, 0, 0x021a, 0x2bff, 0xfe3c, 0x4d5e);

enum Observed {
    Open(SocketSpec),
    Close,
    Send { dst: SocketAddr, frame: Vec<u8> },
    Intent(Intent),
    State(State),
    Warning(String),
    Failed,
}

impl Observed {
    fn intent(&self) -> Option<Intent> {
        match self {
            &Self::Intent(intent) => Some(intent),
            &Self::Open(_)
            | &Self::Close
            | &Self::Send { .. }
            | &Self::State(_)
            | &Self::Warning(_)
            | &Self::Failed => None,
        }
    }
}

fn link() -> LinkInfo {
    LinkInfo {
        name: Name::new("eth0").expect("valid name"),
        index: 2,
        mac: Mac::new(MAC),
        state: LinkState::Up,
    }
}

fn link_with(state: LinkState) -> LinkInfo {
    LinkInfo { state, ..link() }
}

fn new_method() -> Slaac {
    new_method_with_dns(&[])
}

fn new_method_with_dns(dns: &[Ipv6Addr]) -> Slaac {
    let config = Ipv6Config {
        addresses: Vec::new(),
        gateway: None,
        dns: dns.to_vec(),
        autoconf: true,
    };

    Slaac::new(link(), config)
}

fn at(now: Instant, secs: u64) -> Instant {
    now.checked_add(Duration::from_secs(secs))
        .expect("representable instant")
}

fn observe(method: &mut Slaac, now: Instant, input: Input<'_>) -> Option<Observed> {
    match method.step(now, SystemTime::UNIX_EPOCH, input) {
        Step::Wait => None,
        Step::Effect(effect) => Some(convert(effect)),
    }
}

fn convert(effect: Effect<'_>) -> Observed {
    match effect {
        Effect::OpenSocket(spec) => Observed::Open(spec),
        Effect::CloseSocket { .. } => Observed::Close,
        Effect::Send { dst, frame, .. } => {
            let Some(dst) = dst else {
                panic!("ICMPv6 sends carry a destination");
            };
            Observed::Send {
                dst,
                frame: frame.to_vec(),
            }
        }
        Effect::Emit(event) => convert_event(event),
        Effect::RequestEntropy => panic!("unexpected entropy request"),
    }
}

fn convert_event(event: Event) -> Observed {
    match event {
        Event::Intent(intent) => Observed::Intent(intent),
        Event::State(state) => Observed::State(state),
        Event::Warning(text) => Observed::Warning(text),
        Event::Failed(Failure::Io(_) | Failure::Protocol(_)) => Observed::Failed,
    }
}

fn poll(method: &mut Slaac, now: Instant) -> Option<Observed> {
    observe(method, now, Input::Poll)
}

fn drain(method: &mut Slaac, now: Instant) -> Vec<Observed> {
    let mut effects = Vec::new();
    while let Some(effect) = poll(method, now) {
        effects.push(effect);
    }

    effects
}

fn start(method: &mut Slaac, now: Instant) {
    let opened = observe(method, now, Input::Command(Command::Start));
    assert!(matches!(opened, Some(Observed::Open(_))));
    let opened = observe(
        method,
        now,
        Input::SocketOpened {
            slot: SocketSlot::Primary,
        },
    );
    assert!(matches!(opened, Some(Observed::State(State::Acquiring))));
    let solicitation = poll(method, now);
    assert!(matches!(solicitation, Some(Observed::Send { .. })));
}

fn feed(method: &mut Slaac, now: Instant, packet: &[u8], source: Ipv6Addr) -> Vec<Observed> {
    let mut effects = Vec::new();
    let first = observe(
        method,
        now,
        Input::Frame {
            slot: SocketSlot::Primary,
            bytes: packet,
            source: Some(IpAddr::V6(source)),
        },
    );
    effects.extend(first);
    effects.extend(drain(method, now));

    effects
}

fn advertisement(router_lifetime: u16, prefix: Option<(u32, u32)>, dns: &[Ipv6Addr]) -> Vec<u8> {
    advertisement_with(PREFIX, router_lifetime, prefix, dns, 3600)
}

fn advertisement_with(
    network: Ipv6Addr,
    router_lifetime: u16,
    prefix: Option<(u32, u32)>,
    dns: &[Ipv6Addr],
    dns_lifetime: u32,
) -> Vec<u8> {
    let prefixes: Vec<PrefixInformation> = prefix
        .map(|(valid_lifetime, preferred_lifetime)| PrefixInformation {
            prefix: network,
            prefix_len: 64,
            on_link: true,
            autonomous: true,
            valid_lifetime,
            preferred_lifetime,
        })
        .into_iter()
        .collect();
    let content = AdvertisementHeader {
        hop_limit: 64,
        managed_flag: false,
        other_flag: false,
        router_lifetime,
        reachable_time: 0,
        retrans_timer: 0,
    };
    let content = Advertisement {
        header: content,
        link_layer: None,
        mtu: None,
        prefixes: &prefixes,
        dns_lifetime,
        dns_servers: dns,
    };
    let mut buffer = [0_u8; 256];
    let written = build_advertisement(ROUTER, ROUTER, &content, &mut buffer)
        .expect("buffer should hold the advertisement");

    written.to_vec()
}

fn bind(method: &mut Slaac, now: Instant) {
    start(method, now);
    let packet = advertisement(1800, Some((7200, 3600)), &[]);
    let effects = feed(method, now, &packet, ROUTER);
    assert_eq!(effects.len(), 3);
}

fn first_intent(effects: &[Observed]) -> Intent {
    let Some(intent) = effects.first().and_then(Observed::intent) else {
        panic!("expected an intent");
    };

    intent
}

fn second_intent(effects: &[Observed]) -> Intent {
    let Some(intent) = effects.get(1).and_then(Observed::intent) else {
        panic!("expected an intent");
    };

    intent
}

fn send_count(effects: &[Observed]) -> usize {
    effects
        .iter()
        .filter(|effect| matches!(effect, Observed::Send { .. }))
        .count()
}

#[test]
fn start_opens_a_raw_icmpv6_socket() {
    // ARRANGE
    let now = Instant::now();
    let mut method = new_method();

    // ACT
    let first = observe(&mut method, now, Input::Command(Command::Start));

    // ASSERT
    let Some(Observed::Open(spec)) = first else {
        panic!("expected an open socket effect");
    };
    assert_eq!(spec.slot, SocketSlot::Primary);
    assert_eq!(spec.kind, SocketKind::RawIcmpv6);
    assert_eq!(spec.link, "eth0");
}

#[test]
fn opened_socket_sends_one_router_solicitation_to_all_routers() {
    // ARRANGE
    let now = Instant::now();
    let mut method = new_method();
    let _opened = observe(&mut method, now, Input::Command(Command::Start));

    // ACT
    let acquiring = observe(
        &mut method,
        now,
        Input::SocketOpened {
            slot: SocketSlot::Primary,
        },
    );
    let solicitation = poll(&mut method, now);

    // ASSERT
    assert!(matches!(acquiring, Some(Observed::State(State::Acquiring))));
    let Some(Observed::Send { dst, frame }) = solicitation else {
        panic!("expected a solicitation send");
    };
    assert_eq!(dst, SocketAddr::V6(SocketAddrV6::new(ALL_ROUTERS, 0, 0, 0)));
    assert_eq!(frame.first().copied(), Some(ROUTER_SOLICITATION));
    assert_eq!(
        router_solicitation(&frame)
            .expect("solicitation should parse")
            .count(),
        0
    );
}

#[test]
fn router_advertisement_configures_address_route_and_binds() {
    // ARRANGE
    let now = Instant::now();
    let mut method = new_method();
    start(&mut method, now);
    let packet = advertisement(1800, Some((7200, 3600)), &[]);

    // ACT
    let effects = feed(&mut method, now, &packet, ROUTER);

    // ASSERT
    assert!(matches!(
        first_intent(&effects),
        Intent::Configure(config)
            if config.address == IpAddr::V6(ADDRESS) && config.prefix_len == 64
    ));
    assert!(matches!(
        second_intent(&effects),
        Intent::EnsureDefaultRoute(IpAddr::V6(ROUTER))
    ));
    assert!(matches!(
        effects.get(2),
        Some(Observed::State(State::Bound))
    ));
    assert!(matches!(
        method.view(),
        Some(View { addresses, routers, dns })
            if addresses == [(ADDRESS, 64)] && routers == vec![ROUTER] && dns.is_empty()
    ));
}

#[test]
fn missing_router_advertisement_warns_after_three_attempts() {
    // ARRANGE
    let now = Instant::now();
    let mut method = new_method();
    start(&mut method, now);

    // ACT
    let first_wake = method.next_wake(now);
    let second = observe(&mut method, at(now, 4), Input::Timer);
    let third = observe(&mut method, at(now, 8), Input::Timer);
    let exhausted = observe(&mut method, at(now, 12), Input::Timer);
    let degraded = poll(&mut method, at(now, 12));

    // ASSERT
    assert_eq!(first_wake, Some(at(now, 4)));
    assert!(matches!(second, Some(Observed::Send { .. })));
    assert!(matches!(third, Some(Observed::Send { .. })));
    assert!(matches!(
        exhausted,
        Some(Observed::Warning(text)) if text.contains("continuing to listen")
    ));
    assert!(matches!(degraded, Some(Observed::State(State::Degraded))));
    assert!(method.next_wake(at(now, 12)).is_none());
    assert!(method.view().is_none());
}

#[test]
fn solicitation_schedule_is_one_send_per_interval() {
    // ARRANGE
    let now = Instant::now();
    let mut method = new_method();
    start(&mut method, now);
    let mut sends = 1;

    // ACT
    let second = observe(&mut method, at(now, 4), Input::Timer);
    sends += usize::from(matches!(second, Some(Observed::Send { .. })));
    let third = observe(&mut method, at(now, 8), Input::Timer);
    sends += usize::from(matches!(third, Some(Observed::Send { .. })));
    let wake = method.next_wake(at(now, 8));

    // ASSERT
    assert_eq!(sends, 3);
    assert_eq!(wake, Some(at(now, 12)));
}

#[test]
fn stop_deconfigures_removes_route_and_closes_the_socket() {
    // ARRANGE
    let now = Instant::now();
    let mut method = new_method();
    bind(&mut method, now);

    // ACT
    let effects = {
        let first = observe(&mut method, now, Input::Command(Command::Stop));
        let mut effects = first.into_iter().collect::<Vec<_>>();
        effects.extend(drain(&mut method, now));
        effects
    };

    // ASSERT
    assert!(matches!(
        first_intent(&effects),
        Intent::Deconfigure(config)
            if config.address == IpAddr::V6(ADDRESS) && config.prefix_len == 64
    ));
    assert!(matches!(
        second_intent(&effects),
        Intent::RemoveDefaultRoute(IpAddr::V6(ROUTER))
    ));
    assert!(matches!(
        effects.get(2),
        Some(Observed::State(State::Stopped))
    ));
    assert!(matches!(effects.get(3), Some(Observed::Close)));
    assert!(method.next_wake(now).is_none());
    assert!(method.view().is_none());
}

#[test]
fn reconcile_reemits_configuration_without_soliciting() {
    // ARRANGE
    let now = Instant::now();
    let mut method = new_method();
    bind(&mut method, now);

    // ACT
    let effects = {
        let first = observe(&mut method, now, Input::Command(Command::Reconcile));
        let mut effects = first.into_iter().collect::<Vec<_>>();
        effects.extend(drain(&mut method, now));
        effects
    };

    // ASSERT
    assert_eq!(effects.len(), 2);
    assert!(matches!(
        first_intent(&effects),
        Intent::Configure(config) if config.address == IpAddr::V6(ADDRESS)
    ));
    assert!(matches!(
        second_intent(&effects),
        Intent::EnsureDefaultRoute(IpAddr::V6(ROUTER))
    ));
    assert_eq!(send_count(&effects), 0);
}

#[test]
fn reconcile_without_configuration_does_nothing() {
    // ARRANGE
    let now = Instant::now();
    let mut method = new_method();
    start(&mut method, now);

    // ACT
    let effects = {
        let first = observe(&mut method, now, Input::Command(Command::Reconcile));
        let mut effects = first.into_iter().collect::<Vec<_>>();
        effects.extend(drain(&mut method, now));
        effects
    };

    // ASSERT
    assert!(effects.is_empty());
    assert!(method.view().is_none());
}

#[test]
fn link_up_after_suspend_resolicits() {
    // ARRANGE
    let now = Instant::now();
    let mut method = new_method();
    bind(&mut method, now);
    let degraded = observe(&mut method, now, Input::Command(Command::LinkDown));
    assert!(matches!(degraded, Some(Observed::State(State::Degraded))));

    // ACT
    let acquiring = observe(&mut method, now, Input::Command(Command::LinkUp));
    let solicitation = poll(&mut method, now);

    // ASSERT
    assert!(matches!(acquiring, Some(Observed::State(State::Acquiring))));
    assert!(matches!(solicitation, Some(Observed::Send { .. })));
    assert_eq!(method.next_wake(now), Some(at(now, 4)));
}

#[test]
fn preferred_lifetime_expiry_keeps_the_address_deprecated() {
    // ARRANGE
    let now = Instant::now();
    let mut method = new_method();
    start(&mut method, now);
    let packet = advertisement(1800, Some((10, 1)), &[]);
    let _configured = feed(&mut method, now, &packet, ROUTER);

    // ACT
    let effect = observe(&mut method, at(now, 1), Input::Timer);
    let wake = method.next_wake(at(now, 1));

    // ASSERT
    assert!(effect.is_none());
    assert!(matches!(
        method.view(),
        Some(View { addresses, .. }) if addresses == [(ADDRESS, 64)]
    ));
    assert_eq!(wake, Some(at(now, 10)));
}

#[test]
fn router_expiry_removes_the_default_route() {
    // ARRANGE
    let now = Instant::now();
    let mut method = new_method();
    start(&mut method, now);
    let packet = advertisement(1, Some((100, 100)), &[]);
    let _configured = feed(&mut method, now, &packet, ROUTER);

    // ACT
    let effect = observe(&mut method, at(now, 1), Input::Timer);

    // ASSERT
    let Some(Observed::Intent(intent)) = effect else {
        panic!("expected a route removal");
    };
    assert!(matches!(
        intent,
        Intent::RemoveDefaultRoute(IpAddr::V6(ROUTER))
    ));
    assert!(matches!(
        method.view(),
        Some(View { routers, .. }) if routers.is_empty()
    ));
}

#[test]
fn rdnss_update_is_reflected_in_view() {
    // ARRANGE
    let now = Instant::now();
    let mut method = new_method();
    bind(&mut method, now);
    let packet = advertisement(1800, Some((100, 100)), &[DNS]);

    // ACT
    let effects = feed(&mut method, now, &packet, ROUTER);

    // ASSERT
    assert!(effects.is_empty());
    assert!(matches!(
        method.view(),
        Some(View { dns, .. }) if dns == vec![DNS]
    ));
}

#[test]
fn bound_waits_for_the_router_deadline() {
    // ARRANGE
    let now = Instant::now();
    let mut method = new_method();
    bind(&mut method, now);

    // ACT
    let wake = method.next_wake(now);

    // ASSERT
    assert_eq!(wake, Some(at(now, 1800)));
}

#[test]
fn suspended_method_has_no_wake_deadline() {
    // ARRANGE
    let now = Instant::now();
    let mut method = new_method();
    bind(&mut method, now);

    // ACT
    let effect = observe(&mut method, now, Input::Command(Command::LinkDown));
    let wake = method.next_wake(now);

    // ASSERT
    assert!(matches!(effect, Some(Observed::State(State::Degraded))));
    assert!(wake.is_none());
}

#[test]
fn socket_error_is_paced_by_the_back_off() {
    // ARRANGE
    let now = Instant::now();
    let mut method = new_method();
    bind(&mut method, now);
    let error = io::Error::from(io::ErrorKind::NetworkUnreachable);

    // ACT
    let failed = observe(
        &mut method,
        now,
        Input::SocketError {
            slot: SocketSlot::Primary,
            error,
        },
    );
    let wake = method.next_wake(now);
    let retried = observe(&mut method, at(now, 4), Input::Timer);

    // ASSERT
    assert!(matches!(failed, Some(Observed::Failed)));
    assert_eq!(wake, Some(at(now, 4)));
    assert!(retried.is_none());
    assert_eq!(method.next_wake(at(now, 4)), Some(at(now, 1800)));
}

#[test]
fn stop_and_start_clear_the_back_off_and_reopen_the_socket() {
    // ARRANGE
    let now = Instant::now();
    let mut method = new_method();
    bind(&mut method, now);
    let error = io::Error::from(io::ErrorKind::NetworkUnreachable);
    let _failed = observe(
        &mut method,
        now,
        Input::SocketError {
            slot: SocketSlot::Primary,
            error,
        },
    );
    let _stopped = observe(&mut method, now, Input::Command(Command::Stop));
    let _effects: Vec<Observed> = drain(&mut method, now);
    let stopped_wake = method.next_wake(now);

    // ACT
    let reopened = observe(&mut method, at(now, 1), Input::Command(Command::Start));
    let _opened = observe(
        &mut method,
        at(now, 1),
        Input::SocketOpened {
            slot: SocketSlot::Primary,
        },
    );
    let _solicitation = poll(&mut method, at(now, 1));
    let wake = method.next_wake(at(now, 1));

    // ASSERT
    assert!(stopped_wake.is_none());
    assert!(matches!(reopened, Some(Observed::Open(_))));
    assert_eq!(wake, Some(at(now, 5)));
}

#[test]
fn non_router_advertisement_frames_are_ignored() {
    // ARRANGE
    let now = Instant::now();
    let mut method = new_method();
    start(&mut method, now);
    let frame = [NEIGHBOR_SOLICITATION, 0, 0, 0, 0, 0, 0, 0];

    // ACT
    let effects = feed(&mut method, now, &frame, ROUTER);

    // ASSERT
    assert!(effects.is_empty());
    assert!(method.view().is_none());
}

#[test]
fn malformed_router_advertisement_warns_without_configuring() {
    // ARRANGE
    let now = Instant::now();
    let mut method = new_method();
    start(&mut method, now);
    let frame = [ROUTER_ADVERTISEMENT, 0, 0, 0];

    // ACT
    let effects = feed(&mut method, now, &frame, ROUTER);

    // ASSERT
    assert!(matches!(
        effects.first(),
        Some(Observed::Warning(text)) if text.contains("malformed")
    ));
    assert!(method.view().is_none());
}

#[test]
fn router_expiry_then_renumber_deconfigures_replaced_state() {
    // ARRANGE
    let now = Instant::now();
    let mut method = new_method();
    start(&mut method, now);
    let packet = advertisement(1, Some((100, 100)), &[]);
    let _configured = feed(&mut method, now, &packet, ROUTER);
    let expired = observe(&mut method, at(now, 1), Input::Timer);
    assert!(matches!(
        expired,
        Some(Observed::Intent(Intent::RemoveDefaultRoute(IpAddr::V6(
            ROUTER
        ))))
    ));
    let renumbered = advertisement_with(PREFIX_B, 1800, Some((100, 100)), &[], 3600);

    // ACT
    let effects = feed(&mut method, at(now, 1), &renumbered, ROUTER_B);

    // ASSERT
    assert!(matches!(
        first_intent(&effects),
        Intent::Deconfigure(config)
            if config.address == IpAddr::V6(ADDRESS) && config.prefix_len == 64
    ));
    assert!(matches!(
        second_intent(&effects),
        Intent::Configure(config)
            if config.address == IpAddr::V6(ADDRESS_B) && config.prefix_len == 64
    ));
    assert!(matches!(
        effects.get(2).and_then(Observed::intent),
        Some(Intent::EnsureDefaultRoute(IpAddr::V6(ROUTER_B)))
    ));
}

#[test]
fn address_expiry_then_new_router_removes_old_route() {
    // ARRANGE
    let now = Instant::now();
    let mut method = new_method();
    start(&mut method, now);
    let packet = advertisement(100, Some((1, 1)), &[]);
    let _configured = feed(&mut method, now, &packet, ROUTER);
    let deprecated = observe(&mut method, at(now, 1), Input::Timer);
    assert!(deprecated.is_none());
    let expired = observe(&mut method, at(now, 1), Input::Timer);
    assert!(matches!(
        expired,
        Some(Observed::Intent(Intent::Deconfigure(_)))
    ));
    let next = advertisement_with(PREFIX_B, 100, Some((100, 100)), &[], 3600);

    // ACT
    let effects = feed(&mut method, at(now, 1), &next, ROUTER_B);

    // ASSERT
    assert!(matches!(
        first_intent(&effects),
        Intent::RemoveDefaultRoute(IpAddr::V6(ROUTER))
    ));
    assert!(matches!(
        second_intent(&effects),
        Intent::Configure(config)
            if config.address == IpAddr::V6(ADDRESS_B) && config.prefix_len == 64
    ));
    assert!(matches!(
        effects.get(2).and_then(Observed::intent),
        Some(Intent::EnsureDefaultRoute(IpAddr::V6(ROUTER_B)))
    ));
}

#[test]
fn open_failure_is_retried_after_backoff() {
    // ARRANGE
    let now = Instant::now();
    let mut method = new_method();
    let _opened = observe(&mut method, now, Input::Command(Command::Start));
    let error = io::Error::from(io::ErrorKind::NetworkUnreachable);

    // ACT
    let failed = observe(
        &mut method,
        now,
        Input::SocketError {
            slot: SocketSlot::Primary,
            error,
        },
    );
    let wake = method.next_wake(now);
    let reopened = observe(&mut method, at(now, 4), Input::Timer);

    // ASSERT
    assert!(matches!(failed, Some(Observed::Failed)));
    assert_eq!(wake, Some(at(now, 4)));
    assert!(matches!(reopened, Some(Observed::Open(_))));
}

#[test]
fn transient_socket_error_warns_and_backs_off() {
    // ARRANGE
    let now = Instant::now();
    let mut method = new_method();
    let _opened = observe(&mut method, now, Input::Command(Command::Start));
    let error = io::Error::from(io::ErrorKind::WouldBlock);

    // ACT
    let warned = observe(
        &mut method,
        now,
        Input::SocketError {
            slot: SocketSlot::Primary,
            error,
        },
    );
    let wake = method.next_wake(now);

    // ASSERT
    assert!(matches!(warned, Some(Observed::Warning(text)) if text.contains("transient")));
    assert_eq!(wake, Some(at(now, 4)));
}

#[test]
fn unexpected_socket_close_is_recovered() {
    // ARRANGE
    let now = Instant::now();
    let mut method = new_method();
    bind(&mut method, now);

    // ACT
    let closed = observe(
        &mut method,
        now,
        Input::SocketClosed {
            slot: SocketSlot::Primary,
        },
    );
    let wake = method.next_wake(now);
    let reopened = observe(&mut method, at(now, 4), Input::Timer);

    // ASSERT
    assert!(matches!(closed, Some(Observed::Warning(text)) if text.contains("closed")));
    assert_eq!(wake, Some(at(now, 4)));
    assert!(matches!(reopened, Some(Observed::Open(_))));
}

#[test]
fn non_link_local_router_advertisement_is_ignored() {
    // ARRANGE
    let now = Instant::now();
    let mut method = new_method();
    start(&mut method, now);
    let packet = advertisement(1800, Some((7200, 3600)), &[]);

    // ACT
    let effects = feed(&mut method, now, &packet, PREFIX);

    // ASSERT
    assert!(effects.is_empty());
    assert!(method.view().is_none());
}

#[test]
fn link_down_suspends() {
    // ARRANGE
    let now = Instant::now();
    let mut method = new_method();
    bind(&mut method, now);

    // ACT
    let down = observe(&mut method, now, Input::Link(link_with(LinkState::Down)));
    let wake = method.next_wake(now);

    // ASSERT
    assert!(matches!(down, Some(Observed::State(State::Degraded))));
    assert!(wake.is_none());
}

#[test]
fn link_up_while_running_resolicits() {
    // ARRANGE
    let now = Instant::now();
    let mut method = new_method();
    bind(&mut method, now);

    // ACT
    let up = observe(&mut method, now, Input::Link(link_with(LinkState::Up)));
    let solicitation = poll(&mut method, now);

    // ASSERT
    assert!(matches!(up, Some(Observed::State(State::Acquiring))));
    assert!(matches!(solicitation, Some(Observed::Send { .. })));
    assert_eq!(method.next_wake(now), Some(at(now, 4)));
}

#[test]
fn link_down_before_start_does_not_degrade() {
    // ARRANGE
    let now = Instant::now();
    let mut method = new_method();

    // ACT
    let event = observe(&mut method, now, Input::Command(Command::LinkDown));
    let wake = method.next_wake(now);

    // ASSERT
    assert!(event.is_none());
    assert!(wake.is_none());
}

#[test]
fn rdnss_only_advertisement_learns_dns_without_prefix() {
    // ARRANGE
    let now = Instant::now();
    let mut method = new_method();
    start(&mut method, now);
    let packet = advertisement_with(PREFIX, 1800, None, &[DNS], 3600);

    // ACT
    let effects = feed(&mut method, now, &packet, ROUTER);

    // ASSERT
    assert!(matches!(
        effects.first(),
        Some(Observed::Warning(text)) if text.contains("no usable autonomous prefix")
    ));
    assert!(matches!(
        method.view(),
        Some(View { addresses, routers, dns })
            if addresses.is_empty() && routers.is_empty() && dns == vec![DNS]
    ));
}

#[test]
fn configured_fallback_dns_is_learned_without_prefix() {
    // ARRANGE
    let now = Instant::now();
    let mut method = new_method_with_dns(&[FALLBACK]);
    start(&mut method, now);
    let packet = advertisement(1800, None, &[]);

    // ACT
    let _learned = feed(&mut method, now, &packet, ROUTER);

    // ASSERT
    assert!(matches!(
        method.view(),
        Some(View { addresses, dns, .. }) if addresses.is_empty() && dns == vec![FALLBACK]
    ));
}

#[test]
fn dns_lifetime_expiry_falls_back_to_configured_dns() {
    // ARRANGE
    let now = Instant::now();
    let mut method = new_method_with_dns(&[FALLBACK]);
    start(&mut method, now);
    let packet = advertisement_with(PREFIX, 1800, Some((100, 100)), &[DNS], 1);
    let _configured = feed(&mut method, now, &packet, ROUTER);

    // ACT
    let expired = observe(&mut method, at(now, 1), Input::Timer);

    // ASSERT
    assert!(expired.is_none());
    assert!(matches!(
        method.view(),
        Some(View { dns, .. }) if dns == vec![FALLBACK]
    ));
}

#[test]
fn rdnss_lifetime_zero_removes_servers() {
    // ARRANGE
    let now = Instant::now();
    let mut method = new_method();
    start(&mut method, now);
    let learned = advertisement_with(PREFIX, 1800, Some((100, 100)), &[DNS], 3600);
    let _learned = feed(&mut method, now, &learned, ROUTER);
    assert!(matches!(
        method.view(),
        Some(View { dns, .. }) if dns == vec![DNS]
    ));
    let removed = advertisement_with(PREFIX, 1800, Some((100, 100)), &[DNS], 0);

    // ACT
    let _removed = feed(&mut method, now, &removed, ROUTER);

    // ASSERT
    assert!(matches!(
        method.view(),
        Some(View { dns, .. }) if dns.is_empty()
    ));
}

#[test]
fn frames_are_suppressed_during_backoff() {
    // ARRANGE
    let now = Instant::now();
    let mut method = new_method();
    bind(&mut method, now);
    let error = io::Error::from(io::ErrorKind::WouldBlock);
    let _warned = observe(
        &mut method,
        now,
        Input::SocketError {
            slot: SocketSlot::Primary,
            error,
        },
    );
    let packet = advertisement_with(PREFIX, 1800, Some((100, 100)), &[DNS], 3600);

    // ACT
    let effects = feed(&mut method, now, &packet, ROUTER);

    // ASSERT
    assert!(effects.is_empty());
    assert!(matches!(
        method.view(),
        Some(View { dns, .. }) if dns.is_empty()
    ));
}
