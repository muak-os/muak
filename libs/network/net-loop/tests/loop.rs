//! End-to-end driver tests over the mock runtime.

#![cfg(test)]

use core::future::Future;
#[cfg(feature = "ipv6")]
use core::net::Ipv6Addr;
use core::net::{IpAddr, Ipv4Addr};
use core::pin::pin;
use core::task::{Context, Poll, Waker};
use std::io;
use std::time::{Instant, SystemTime};

#[cfg(feature = "ipv4")]
use ip::v4;
#[cfg(feature = "ipv6")]
use ndp::build;
use net_loop::{Loop, Progress, SocketOwner, SocketTable};
use net_runtime::error::{Result, RuntimeError};
use net_runtime::mock::Mock;

#[cfg(feature = "ipv4")]
const DHCP_XID: u32 = 0x0302_0100;
#[cfg(feature = "ipv4")]
const DHCP_SERVER: Ipv4Addr = Ipv4Addr::new(10, 0, 0, 1);
#[cfg(feature = "ipv6")]
const ROUTER: Ipv6Addr = Ipv6Addr::new(0xfe80, 0, 0, 0, 0, 0, 0, 1);
#[cfg(feature = "ipv6")]
const PREFIX_V6: Ipv6Addr = Ipv6Addr::new(0x2001, 0x0db8, 0, 0, 0, 0, 0, 0);

const ADDRESS: Ipv4Addr = Ipv4Addr::new(10, 0, 0, 2);
const PREFIX: u8 = 24;
const DNS: Ipv4Addr = Ipv4Addr::new(9, 9, 9, 9);
const SETTLE: usize = 64;

struct Failing {
    mock: Mock,
}

impl net_runtime::Runtime for Failing {
    type Address = Mock;
    type Clock = Mock;
    type Entropy = Mock;
    type Events = Mock;
    type Icmpv6 = Mock;
    type Link = Self;
    type Packet = Mock;
    type Route = Mock;
    type Udp = Mock;

    fn address(&self) -> &Mock {
        self.mock.address()
    }

    fn clock(&self) -> &Mock {
        self.mock.clock()
    }

    fn entropy(&self) -> &Mock {
        self.mock.entropy()
    }

    fn events(&self) -> &Mock {
        self.mock.events()
    }

    fn icmpv6(&self) -> &Mock {
        self.mock.icmpv6()
    }

    fn link(&self) -> &Self::Link {
        self
    }

    fn packet(&self) -> &Mock {
        self.mock.packet()
    }

    fn route(&self) -> &Mock {
        self.mock.route()
    }

    fn udp(&self) -> &Mock {
        self.mock.udp()
    }
}

impl net_runtime::Link for Failing {
    fn detach(&self, port: &netmodel::Name) -> impl Future<Output = Result<()>> + Send {
        self.mock.detach(port)
    }

    fn ensure_bridge(
        &self,
        name: &netmodel::Name,
        stp: bool,
    ) -> impl Future<Output = Result<()>> + Send {
        self.mock.ensure_bridge(name, stp)
    }

    fn link(
        &self,
        name: &netmodel::Name,
    ) -> impl Future<Output = Result<Option<netmodel::Link>>> + Send {
        self.mock.link(name)
    }

    fn links(&self) -> impl Future<Output = Result<Vec<netmodel::Link>>> + Send {
        std::future::ready(Err(RuntimeError::Io(io::Error::other(
            "link inventory failed",
        ))))
    }

    fn set_down(&self, name: &netmodel::Name) -> impl Future<Output = Result<()>> + Send {
        self.mock.set_down(name)
    }

    fn set_master(
        &self,
        port: &netmodel::Name,
        master: &netmodel::Name,
    ) -> impl Future<Output = Result<()>> + Send {
        self.mock.set_master(port, master)
    }

    fn set_up(&self, name: &netmodel::Name) -> impl Future<Output = Result<()>> + Send {
        self.mock.set_up(name)
    }
}

struct SlowPhy {
    mock: Mock,
}

impl net_runtime::Runtime for SlowPhy {
    type Address = Mock;
    type Clock = Mock;
    type Entropy = Mock;
    type Events = Mock;
    type Icmpv6 = Mock;
    type Link = Self;
    type Packet = Mock;
    type Route = Mock;
    type Udp = Mock;

    fn address(&self) -> &Mock {
        self.mock.address()
    }

    fn clock(&self) -> &Mock {
        self.mock.clock()
    }

    fn entropy(&self) -> &Mock {
        self.mock.entropy()
    }

    fn events(&self) -> &Mock {
        self.mock.events()
    }

    fn icmpv6(&self) -> &Mock {
        self.mock.icmpv6()
    }

    fn link(&self) -> &Self::Link {
        self
    }

    fn packet(&self) -> &Mock {
        self.mock.packet()
    }

    fn route(&self) -> &Mock {
        self.mock.route()
    }

    fn udp(&self) -> &Mock {
        self.mock.udp()
    }
}

impl net_runtime::Link for SlowPhy {
    fn detach(&self, port: &netmodel::Name) -> impl Future<Output = Result<()>> + Send {
        self.mock.detach(port)
    }

    fn ensure_bridge(
        &self,
        name: &netmodel::Name,
        stp: bool,
    ) -> impl Future<Output = Result<()>> + Send {
        self.mock.ensure_bridge(name, stp)
    }

    fn link(
        &self,
        name: &netmodel::Name,
    ) -> impl Future<Output = Result<Option<netmodel::Link>>> + Send {
        self.mock.link(name)
    }

    fn links(&self) -> impl Future<Output = Result<Vec<netmodel::Link>>> + Send {
        self.mock.links()
    }

    fn set_down(&self, name: &netmodel::Name) -> impl Future<Output = Result<()>> + Send {
        self.mock.set_down(name)
    }

    fn set_master(
        &self,
        port: &netmodel::Name,
        master: &netmodel::Name,
    ) -> impl Future<Output = Result<()>> + Send {
        self.mock.set_master(port, master)
    }

    fn set_up(&self, _name: &netmodel::Name) -> impl Future<Output = Result<()>> + Send {
        std::future::ready(Ok(()))
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

fn name(text: &str) -> netmodel::Name {
    netmodel::Name::new(text).expect("valid name")
}

fn link(text: &str, index: u32) -> ip_method::LinkInfo {
    ip_method::LinkInfo {
        name: name(text),
        index,
        mac: mac::Mac::default(),
        state: netmodel::LinkState::Up,
    }
}

fn model_link(text: &str, index: u32, state: netmodel::LinkState) -> netmodel::Link {
    netmodel::Link {
        name: name(text),
        index,
        mac: mac::Mac::default(),
        admin: state.admin(),
        carrier: state.carrier(),
        is_virtual: false,
        master: None,
    }
}

fn owner(interface: &str, slot: ip_method::SocketSlot) -> SocketOwner {
    SocketOwner {
        interface: name(interface),
        family: netmodel::Family::V4,
        slot,
        kind: ip_method::SocketKind::RawL2,
    }
}

fn runtime() -> Mock {
    let mock = Mock::new(Instant::now(), SystemTime::UNIX_EPOCH);
    mock.set_link(model_link("eth0", 2, netmodel::LinkState::Up));

    mock
}

fn ethernet() -> netmodel::InterfaceConfig {
    netmodel::InterfaceConfig {
        name: name("eth0"),
        kind: netmodel::InterfaceKind::Ethernet,
        ipv4: Some(address()),
        ipv6: None,
        bridge: None,
    }
}

fn bridge() -> netmodel::InterfaceConfig {
    netmodel::InterfaceConfig {
        name: name("br0"),
        kind: netmodel::InterfaceKind::Bridge,
        ipv4: Some(address()),
        ipv6: None,
        bridge: Some(netmodel::BridgeConfig {
            port: vec![name("auto")],
            stp: false,
        }),
    }
}

fn address() -> netmodel::Ipv4Config {
    netmodel::Ipv4Config {
        addresses: vec![(ADDRESS, PREFIX)],
        gateway: None,
        dns: Vec::new(),
        dhcp: false,
    }
}

fn config(interfaces: Vec<netmodel::InterfaceConfig>) -> netmodel::Config {
    netmodel::Config {
        ipv6: false,
        dns: Vec::new(),
        interfaces,
    }
}

fn assigned() -> netmodel::Address {
    netmodel::Address {
        address: IpAddr::V4(ADDRESS),
        prefix_len: PREFIX,
    }
}

fn addresses(driver: &Loop<Mock>, interface: &netmodel::Name) -> Vec<netmodel::Address> {
    block_on(net_runtime::Address::list(driver.runtime(), interface)).expect("list addresses")
}

async fn settle<R: net_runtime::Runtime>(driver: &mut Loop<R>, now: Instant, wall: SystemTime) {
    for _ in 0_usize..SETTLE {
        if driver.step(now, wall).await.expect("step") == Progress::Idle {
            return;
        }
    }

    panic!("driver did not settle");
}

async fn boot<R: net_runtime::Runtime>(driver: &mut Loop<R>, now: Instant, wall: SystemTime) {
    driver.start(now).await;
    settle(driver, now, wall).await;
}

fn link_state(driver: &Loop<Mock>, interface: &netmodel::Name) -> netmodel::LinkState {
    block_on(net_runtime::Link::link(driver.runtime(), interface))
        .expect("query link")
        .expect("known link")
        .state()
}

#[test]
fn start_discovers_provisions_and_configures() {
    // ARRANGE
    let now = Instant::now();
    let wall = SystemTime::UNIX_EPOCH;
    let mut driver: Loop<Mock> = Loop::new(runtime(), config(vec![ethernet()]));

    // ACT
    let started = block_on(driver.start(now));
    let restarted = block_on(driver.start(now));
    block_on(settle(&mut driver, now, wall));

    // ASSERT
    assert_eq!(started, Progress::Worked);
    assert_eq!(restarted, Progress::Idle);
    assert_eq!(driver.status().state, net_supervisor::State::Ready);
    assert_eq!(driver.status().owner, Some(name("eth0")));
    assert_eq!(driver.interfaces().len(), 1);
    assert_eq!(driver.interfaces()[0].link().name, name("eth0"));
    assert_eq!(addresses(&driver, &name("eth0")), vec![assigned()]);
}

#[test]
fn boot_journals_discovery_state_and_effects() {
    // ARRANGE
    let now = Instant::now();
    let wall = SystemTime::UNIX_EPOCH;
    let mut driver: Loop<Mock> = Loop::new(runtime(), config(vec![ethernet()]));

    // ACT
    block_on(boot(&mut driver, now, wall));
    let observations: Vec<net_observe::Observation> = driver.drain_observations().collect();

    // ASSERT
    assert!(observations.iter().any(|observation| matches!(
        observation.event,
        net_observe::Event::LinksDiscovered { .. }
    )));
    let transitions: Vec<(net_observe::SupervisorState, net_observe::SupervisorState)> =
        observations
            .iter()
            .filter_map(|observation| {
                let net_observe::Event::StateChanged { from, to } = observation.event else {
                    return None;
                };
                Some((from, to))
            })
            .collect();
    assert_eq!(
        transitions,
        vec![
            (
                net_observe::SupervisorState::Uninitialized,
                net_observe::SupervisorState::Initializing,
            ),
            (
                net_observe::SupervisorState::Initializing,
                net_observe::SupervisorState::Operational,
            ),
            (
                net_observe::SupervisorState::Operational,
                net_observe::SupervisorState::Ready,
            ),
        ]
    );
    assert!(observations.iter().any(|observation| matches!(
        &observation.event,
        net_observe::Event::EffectStarted {
            kind: net_observe::EffectKind::Discover,
            ..
        }
    )));
    assert!(driver.drain_observations().next().is_none());
}

#[test]
fn boot_journals_started_effects_without_finished_effects() {
    // ARRANGE
    let now = Instant::now();
    let wall = SystemTime::UNIX_EPOCH;
    let mut driver: Loop<Mock> = Loop::new(runtime(), config(vec![ethernet()]));

    // ACT
    block_on(boot(&mut driver, now, wall));
    let observations: Vec<net_observe::Observation> = driver.drain_observations().collect();

    // ASSERT
    assert!(
        observations.iter().any(|observation| matches!(
            observation.event,
            net_observe::Event::EffectStarted { .. }
        ))
    );
    assert!(observations.iter().all(|observation| !matches!(
        observation.event,
        net_observe::Event::EffectFinished { .. }
    )));
}

#[test]
fn boot_journals_binding_and_address_assignment() {
    // ARRANGE
    let now = Instant::now();
    let wall = SystemTime::UNIX_EPOCH;
    let mut driver: Loop<Mock> = Loop::new(runtime(), config(vec![ethernet()]));

    // ACT
    block_on(boot(&mut driver, now, wall));
    let observations: Vec<net_observe::Observation> = driver.drain_observations().collect();

    // ASSERT
    assert!(observations.iter().any(|observation| matches!(
        observation.event,
        net_observe::Event::InterfaceState {
            family: netmodel::Family::V4,
            state: netmodel::MethodState::Bound,
            ..
        }
    )));
    assert!(observations.iter().any(|observation| matches!(
        observation.event,
        net_observe::Event::AddressAssigned { .. }
    )));
}

#[test]
fn down_link_is_activated_before_the_carrier_probe() {
    // ARRANGE
    let now = Instant::now();
    let wall = SystemTime::UNIX_EPOCH;
    let mock = Mock::new(now, wall);
    mock.set_link(model_link("eth0", 2, netmodel::LinkState::Down));
    let mut driver: Loop<Mock> = Loop::new(mock, config(vec![ethernet()]));

    // ACT
    block_on(boot(&mut driver, now, wall));

    // ASSERT
    assert_eq!(driver.status().state, net_supervisor::State::Ready);
    assert_eq!(driver.status().primary, Some(name("eth0")));
    assert_eq!(
        driver.interfaces()[0].snapshot().link,
        netmodel::LinkState::Up
    );
    assert_eq!(link_state(&driver, &name("eth0")), netmodel::LinkState::Up);
    assert_eq!(addresses(&driver, &name("eth0")), vec![assigned()]);
}

#[test]
fn bridge_setup_enslaves_the_port_and_takes_ownership() {
    // ARRANGE
    let now = Instant::now();
    let mut driver: Loop<Mock> = Loop::new(runtime(), config(vec![bridge(), ethernet()]));

    // ACT
    block_on(boot(&mut driver, now, SystemTime::UNIX_EPOCH));

    // ASSERT
    let status = driver.status();
    assert_eq!(status.primary, Some(name("eth0")));
    assert_eq!(status.owner, Some(name("br0")));
    assert_eq!(driver.runtime().bridge_stp(&name("br0")), Some(false));
    let names: Vec<netmodel::Name> = driver
        .interfaces()
        .iter()
        .map(|controller| controller.link().name)
        .collect();
    assert_eq!(names, vec![name("eth0"), name("br0")]);
    let port = block_on(net_runtime::Link::link(driver.runtime(), &name("eth0")))
        .expect("query port")
        .expect("known port");
    let master = block_on(net_runtime::Link::link(driver.runtime(), &name("br0")))
        .expect("query bridge")
        .expect("known bridge");
    assert_eq!(port.master, Some(master.index));
    assert_eq!(addresses(&driver, &name("br0")), vec![assigned()]);
}

#[test]
fn published_snapshot_updates_the_owner_link_state() {
    // ARRANGE
    let now = Instant::now();
    let mut driver: Loop<Mock> = Loop::new(runtime(), config(vec![ethernet()]));
    block_on(boot(&mut driver, now, SystemTime::UNIX_EPOCH));

    // ACT
    let status = driver.status();

    // ASSERT
    assert_eq!(status.owner, Some(name("eth0")));
    assert_eq!(status.owner_link, Some(netmodel::LinkState::Up));
    assert_eq!(
        driver.interfaces()[0].snapshot().link,
        netmodel::LinkState::Up
    );
}

#[test]
fn supervisor_effect_failure_is_fed_back_without_aborting() {
    // ARRANGE
    let now = Instant::now();
    let runtime = Failing { mock: runtime() };
    let mut driver: Loop<Failing> = Loop::new(runtime, config(Vec::new()));

    // ACT
    let started = block_on(driver.start(now));
    block_on(settle(&mut driver, now, SystemTime::UNIX_EPOCH));
    let failures: Vec<net_observe::Observation> = driver
        .drain_observations()
        .filter(|observation| matches!(observation.event, net_observe::Event::EffectFailed { .. }))
        .collect();

    // ASSERT
    assert_eq!(started, Progress::Worked);
    assert_eq!(driver.status().state, net_supervisor::State::Degraded);
    assert_eq!(failures.len(), 1, "the failure must be recorded once");
    assert_eq!(failures[0].level, net_observe::Level::Warn);
    assert!(matches!(
        failures[0].event,
        net_observe::Event::EffectFailed {
            kind: net_observe::EffectKind::Discover,
            ..
        }
    ));
}

#[test]
fn degraded_probe_recovers_when_the_link_comes_up_on_retry() {
    // ARRANGE
    let now = Instant::now();
    let wall = SystemTime::UNIX_EPOCH;
    let mock = Mock::new(now, wall);
    mock.set_link(model_link("eth0", 2, netmodel::LinkState::NoCarrier));
    let mut driver: Loop<Mock> = Loop::new(mock, config(vec![ethernet()]));

    // ACT
    block_on(boot(&mut driver, now, wall));
    let degraded = driver.status().state;
    let retry = driver.next_wake(now).expect("retry deadline");
    driver
        .runtime()
        .set_link(model_link("eth0", 2, netmodel::LinkState::Up));
    block_on(settle(&mut driver, retry, wall));

    // ASSERT
    assert_eq!(degraded, net_supervisor::State::Degraded);
    assert_eq!(driver.status().state, net_supervisor::State::Ready);
    assert_eq!(driver.status().primary, Some(name("eth0")));
}

#[test]
fn slow_phy_link_up_event_recovers_the_supervisor() {
    // ARRANGE
    let now = Instant::now();
    let wall = SystemTime::UNIX_EPOCH;
    let mock = Mock::new(now, wall);
    mock.set_link(model_link("eth0", 2, netmodel::LinkState::Down));
    let mut driver: Loop<SlowPhy> = Loop::new(SlowPhy { mock }, config(vec![ethernet()]));

    // ACT
    block_on(boot(&mut driver, now, wall));
    let degraded = driver.status().state;
    driver
        .runtime()
        .mock
        .set_link(model_link("eth0", 2, netmodel::LinkState::Up));
    driver.runtime().mock.push_event(netmodel::LinkEvent::Up {
        name: name("eth0"),
        index: 2,
    });
    block_on(driver.wait_event()).expect("wait event");
    block_on(settle(&mut driver, now, wall));

    // ASSERT
    assert_eq!(degraded, net_supervisor::State::Degraded);
    assert_eq!(driver.status().state, net_supervisor::State::Ready);
    assert_eq!(driver.status().primary, Some(name("eth0")));
}

#[test]
fn link_event_reaches_the_controller_and_the_supervisor() {
    // ARRANGE
    let now = Instant::now();
    let wall = SystemTime::UNIX_EPOCH;
    let mut driver: Loop<Mock> = Loop::new(runtime(), config(vec![ethernet()]));
    block_on(boot(&mut driver, now, wall));
    driver
        .runtime()
        .set_link(model_link("eth0", 2, netmodel::LinkState::Down));
    driver.runtime().push_event(netmodel::LinkEvent::Down {
        name: name("eth0"),
        index: 2,
    });

    // ACT
    block_on(driver.wait_event()).expect("wait event");
    block_on(settle(&mut driver, now, wall));

    // ASSERT
    assert_eq!(
        driver.interfaces()[0].snapshot().link,
        netmodel::LinkState::Down
    );
    assert_eq!(driver.status().processed_events, 1);
    assert_eq!(
        link_state(&driver, &name("eth0")),
        netmodel::LinkState::Down
    );
}

#[test]
fn deleted_link_skips_the_interface_transition() {
    // ARRANGE
    let now = Instant::now();
    let wall = SystemTime::UNIX_EPOCH;
    let mut driver: Loop<Mock> = Loop::new(runtime(), config(vec![ethernet()]));
    block_on(boot(&mut driver, now, wall));
    driver.runtime().push_event(netmodel::LinkEvent::Deleted {
        name: name("eth0"),
        index: 2,
    });

    // ACT
    block_on(driver.wait_event()).expect("wait event");
    block_on(settle(&mut driver, now, wall));

    // ASSERT
    assert_eq!(driver.status().processed_events, 1);
    assert_eq!(
        driver.interfaces()[0].snapshot().link,
        netmodel::LinkState::Up
    );
}

#[test]
fn resolver_write_is_captured_by_the_loop() {
    // ARRANGE
    let now = Instant::now();
    let mut announced = config(vec![ethernet()]);
    announced.dns = vec![IpAddr::V4(DNS)];
    let mut driver: Loop<Mock> = Loop::new(runtime(), announced);

    // ACT
    block_on(boot(&mut driver, now, SystemTime::UNIX_EPOCH));

    // ASSERT
    assert_eq!(driver.resolver().0, [DNS]);
    assert_eq!(driver.resolver().1.len(), 0);
}

#[test]
fn reconcile_reaches_the_interface_controller() {
    // ARRANGE
    let now = Instant::now();
    let wall = SystemTime::UNIX_EPOCH;
    let mut driver: Loop<Mock> = Loop::new(runtime(), config(vec![bridge(), ethernet()]));
    block_on(boot(&mut driver, now, wall));
    block_on(net_runtime::Address::remove(
        driver.runtime(),
        &name("br0"),
        assigned(),
    ))
    .expect("remove address");
    driver.runtime().push_event(netmodel::LinkEvent::Up {
        name: name("eth0"),
        index: 2,
    });

    // ACT
    block_on(driver.wait_event()).expect("wait event");
    block_on(settle(&mut driver, now, wall));

    // ASSERT
    assert_eq!(addresses(&driver, &name("br0")), vec![assigned()]);
}

#[test]
fn satisfied_reconcile_only_journals_debug_trace() {
    // ARRANGE
    let now = Instant::now();
    let wall = SystemTime::UNIX_EPOCH;
    let mut driver: Loop<Mock> = Loop::new(runtime(), config(vec![bridge(), ethernet()]));
    block_on(boot(&mut driver, now, wall));
    drop(driver.drain_observations());
    let deadline = driver.next_wake(now).expect("reconcile deadline");

    // ACT
    block_on(settle(&mut driver, deadline, wall));
    let observations: Vec<net_observe::Observation> = driver.drain_observations().collect();

    // ASSERT
    assert!(
        !observations.is_empty(),
        "the satisfied reconcile should still trace its work"
    );
    assert!(
        observations
            .iter()
            .all(|observation| observation.level == net_observe::Level::Debug),
        "a satisfied reconcile must not emit operator-level observations"
    );
    assert!(observations.iter().any(|observation| matches!(
        &observation.event,
        net_observe::Event::EffectStarted {
            kind: net_observe::EffectKind::ReconcileInterface,
            target,
        } if *target == Some(name("br0"))
    )));
    assert!(observations.iter().all(|observation| !matches!(
        observation.event,
        net_observe::Event::BridgeReady { .. }
            | net_observe::Event::Enslaved { .. }
            | net_observe::Event::StateChanged { .. }
            | net_observe::Event::InterfaceState { .. }
    )));
}

#[test]
fn wait_event_handles_one_event() {
    // ARRANGE
    let now = Instant::now();
    let wall = SystemTime::UNIX_EPOCH;
    let mut driver: Loop<Mock> = Loop::new(runtime(), config(Vec::new()));
    block_on(boot(&mut driver, now, wall));
    driver.runtime().push_event(netmodel::LinkEvent::Up {
        name: name("eth0"),
        index: 2,
    });

    // ACT
    let waited = block_on(driver.wait_event()).expect("wait event");
    block_on(settle(&mut driver, now, wall));

    // ASSERT
    assert_eq!(waited, Progress::Worked);
    assert_eq!(driver.status().processed_events, 1);
}

#[test]
fn link_event_updates_the_controller_before_the_supervisor() {
    // ARRANGE
    let now = Instant::now();
    let wall = SystemTime::UNIX_EPOCH;
    let mut driver: Loop<Mock> = Loop::new(runtime(), config(vec![ethernet()]));
    block_on(boot(&mut driver, now, wall));
    driver
        .runtime()
        .set_link(model_link("eth0", 2, netmodel::LinkState::Down));
    driver.runtime().push_event(netmodel::LinkEvent::Down {
        name: name("eth0"),
        index: 2,
    });
    block_on(driver.wait_event()).expect("wait event");

    // ACT
    let progress = block_on(driver.step(now, wall)).expect("step");

    // ASSERT
    assert_eq!(progress, Progress::Worked);
    assert_eq!(
        driver.interfaces()[0].snapshot().link,
        netmodel::LinkState::Down
    );
}

#[test]
fn port_carrier_down_reaches_the_bridge_owner() {
    // ARRANGE
    let now = Instant::now();
    let wall = SystemTime::UNIX_EPOCH;
    let mut driver: Loop<Mock> = Loop::new(runtime(), config(vec![bridge(), ethernet()]));
    block_on(boot(&mut driver, now, wall));
    let boot_link = driver.status().owner_link;
    driver
        .runtime()
        .set_link(model_link("eth0", 2, netmodel::LinkState::Down));
    driver.runtime().push_event(netmodel::LinkEvent::Down {
        name: name("eth0"),
        index: 2,
    });
    block_on(driver.wait_event()).expect("wait event");

    // ACT
    block_on(settle(&mut driver, now, wall));
    let down = driver.status();
    driver
        .runtime()
        .set_link(model_link("eth0", 2, netmodel::LinkState::Up));
    driver.runtime().push_event(netmodel::LinkEvent::Up {
        name: name("eth0"),
        index: 2,
    });
    block_on(driver.wait_event()).expect("wait event");
    block_on(settle(&mut driver, now, wall));
    let up = driver.status();

    // ASSERT
    assert_eq!(boot_link, Some(netmodel::LinkState::Up));
    assert_eq!(down.owner_link, Some(netmodel::LinkState::Down));
    assert_eq!(down.state, net_supervisor::State::Degraded);
    assert_eq!(up.owner_link, Some(netmodel::LinkState::Up));
    assert_eq!(up.state, net_supervisor::State::Operational);
}

#[test]
fn port_carrier_loss_degrades_and_recovers_the_bridge_owner() {
    // ARRANGE
    let now = Instant::now();
    let wall = SystemTime::UNIX_EPOCH;
    let mut driver: Loop<Mock> = Loop::new(runtime(), config(vec![bridge(), ethernet()]));
    block_on(boot(&mut driver, now, wall));
    let boot_link = driver.status().owner_link;
    driver
        .runtime()
        .set_link(model_link("eth0", 2, netmodel::LinkState::NoCarrier));
    driver.runtime().push_event(netmodel::LinkEvent::Down {
        name: name("eth0"),
        index: 2,
    });
    block_on(driver.wait_event()).expect("wait event");

    // ACT
    block_on(settle(&mut driver, now, wall));
    let lost = driver.status();
    driver
        .runtime()
        .set_link(model_link("eth0", 2, netmodel::LinkState::Up));
    driver.runtime().push_event(netmodel::LinkEvent::Up {
        name: name("eth0"),
        index: 2,
    });
    block_on(driver.wait_event()).expect("wait event");
    block_on(settle(&mut driver, now, wall));
    let recovered = driver.status();

    // ASSERT
    assert_eq!(boot_link, Some(netmodel::LinkState::Up));
    assert_eq!(lost.owner_link, Some(netmodel::LinkState::NoCarrier));
    assert_eq!(lost.state, net_supervisor::State::Degraded);
    assert_eq!(recovered.owner_link, Some(netmodel::LinkState::Up));
    assert_eq!(recovered.state, net_supervisor::State::Operational);
}

#[test]
fn due_reconcile_produces_work_and_advances_the_wake() {
    // ARRANGE
    let now = Instant::now();
    let wall = SystemTime::UNIX_EPOCH;
    let mut driver: Loop<Mock> = Loop::new(runtime(), config(vec![ethernet()]));
    block_on(boot(&mut driver, now, wall));
    let deadline = driver.next_wake(now).expect("reconcile deadline");
    let mut worked = false;

    // ACT
    for _ in 0_usize..SETTLE {
        let progress = block_on(driver.step(deadline, wall)).expect("step");
        if progress == Progress::Worked {
            worked = true;
        }
        if progress == Progress::Idle {
            break;
        }
    }
    let next = driver.next_wake(deadline).expect("next deadline");

    // ASSERT
    assert!(deadline > now);
    assert!(worked);
    assert!(next > deadline);
}

#[test]
fn due_retry_produces_work_and_advances_the_wake() {
    // ARRANGE
    let now = Instant::now();
    let wall = SystemTime::UNIX_EPOCH;
    let mut driver: Loop<Failing> = Loop::new(Failing { mock: runtime() }, config(Vec::new()));
    block_on(driver.start(now));
    block_on(settle(&mut driver, now, wall));
    let deadline = driver.next_wake(now).expect("retry deadline");

    // ACT
    let worked = block_on(driver.step(deadline, wall)).expect("step");
    for _ in 0_usize..SETTLE {
        if block_on(driver.step(deadline, wall)).expect("step") == Progress::Idle {
            break;
        }
    }
    let next = driver.next_wake(deadline).expect("next deadline");

    // ASSERT
    assert!(deadline > now);
    assert_eq!(worked, Progress::Worked);
    assert!(next > deadline);
}

#[test]
fn next_wake_keeps_the_supervisor_deadline_when_a_controller_has_none() {
    // ARRANGE
    let now = Instant::now();
    let mut driver: Loop<Mock> = Loop::new(runtime(), config(Vec::new()));
    block_on(boot(&mut driver, now, SystemTime::UNIX_EPOCH));

    // ACT
    let wake = driver.next_wake(now);

    // ASSERT
    assert!(wake.is_some_and(|deadline| deadline > now));
}

#[test]
fn new_loop_is_idle_and_empty() {
    // ARRANGE
    let now = Instant::now();
    let mut driver: Loop<Mock> = Loop::new(runtime(), netmodel::Config::default());

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
    let mut driver: Loop<Mock> = Loop::new(runtime(), netmodel::Config::default());

    // ACT
    driver.add_interface(link("eth1", 3));
    let controllers = driver.interfaces();

    // ASSERT
    assert_eq!(controllers.len(), 1);
    assert_eq!(controllers[0].link().index, 3);
    assert!(driver.next_wake(now).is_none());
}

#[cfg(feature = "ipv4")]
fn dhcp_interface() -> netmodel::InterfaceConfig {
    netmodel::InterfaceConfig {
        name: name("eth0"),
        kind: netmodel::InterfaceKind::Ethernet,
        ipv4: Some(netmodel::Ipv4Config {
            dhcp: true,
            ..netmodel::Ipv4Config::default()
        }),
        ipv6: None,
        bridge: None,
    }
}

#[cfg(feature = "ipv6")]
fn slaac_interface() -> netmodel::InterfaceConfig {
    netmodel::InterfaceConfig {
        name: name("eth0"),
        kind: netmodel::InterfaceKind::Ethernet,
        ipv4: None,
        ipv6: Some(netmodel::Ipv6Config {
            autoconf: true,
            ..netmodel::Ipv6Config::default()
        }),
        bridge: None,
    }
}

#[cfg(any(feature = "ipv4", feature = "ipv6"))]
fn open_socket(
    driver: &Loop<Mock>,
    family: netmodel::Family,
    slot: ip_method::SocketSlot,
) -> net_runtime::SocketId {
    driver
        .sockets()
        .find(&name("eth0"), family, slot)
        .expect("open socket")
}

#[cfg(feature = "ipv4")]
fn dhcp_payload(frame: &[u8]) -> &[u8] {
    v4::udp::unwrap(frame).expect("DHCP frame").2
}

#[cfg(feature = "ipv4")]
fn dhcp_reply(xid: u32, kind: u8) -> Vec<u8> {
    let mut buffer = [0_u8; 1500];
    let mac = mac::Mac::default().into_array();
    let header = dhcp_proto::header::build_with(&mut buffer, xid, mac, Ipv4Addr::UNSPECIFIED, 0)
        .expect("DHCP header fits");
    let mut payload = header.to_vec();
    payload[16..20].copy_from_slice(&ADDRESS.octets());
    payload.extend_from_slice(&[
        53, 1, kind, 54, 4, 10, 0, 0, 1, 1, 4, 255, 255, 255, 0, 3, 4, 10, 0, 0, 1, 51, 4, 0, 0, 0,
        100, 255,
    ]);
    let source = v4::Addr {
        ip: DHCP_SERVER,
        port: dhcp_proto::DHCP_SERVER_PORT,
    };
    let destination = v4::Addr {
        ip: Ipv4Addr::BROADCAST,
        port: dhcp_proto::DHCP_CLIENT_PORT,
    };
    let mut frame = [0_u8; 1536];
    let written = v4::udp::wrap(&payload, source, destination, &mut frame)
        .expect("DHCP frame fits")
        .len();

    frame[..written].to_vec()
}

#[cfg(feature = "ipv4")]
#[test]
fn dhcp_exchange_binds_a_lease_through_the_socket_path() {
    // ARRANGE
    let now = Instant::now();
    let wall = SystemTime::UNIX_EPOCH;
    let mut driver: Loop<Mock> = Loop::new(runtime(), config(vec![dhcp_interface()]));
    block_on(boot(&mut driver, now, wall));
    let id = open_socket(
        &driver,
        netmodel::Family::V4,
        ip_method::SocketSlot::Primary,
    );
    let kind = driver.sockets().get(id).map(|owner| owner.kind);
    let discover = driver.runtime().sent(id).expect("sent frames");
    let offer = dhcp_reply(DHCP_XID, dhcp_proto::message_type::OFFER);

    // ACT
    driver
        .runtime()
        .push_inbound(id, &offer)
        .expect("queue offer");
    let offered = block_on(driver.wait_event()).expect("wait offer");
    block_on(settle(&mut driver, now, wall));
    let sent = driver.runtime().sent(id).expect("sent frames");
    let xid = sent
        .get(1)
        .map(|frame| dhcp_proto::xid::extract(dhcp_payload(frame)))
        .expect("request sent");
    let ack = dhcp_reply(xid, dhcp_proto::message_type::ACK);
    driver.runtime().push_inbound(id, &ack).expect("queue ack");
    let acknowledged = block_on(driver.wait_event()).expect("wait ack");
    block_on(settle(&mut driver, now, wall));
    let lease = driver.drain_observations().find_map(|observation| {
        let net_observe::Event::DhcpLease { lease, .. } = observation.event else {
            return None;
        };
        Some((observation.level, lease))
    });

    // ASSERT
    assert_eq!(kind, Some(ip_method::SocketKind::RawL2));
    assert_eq!(
        discover.len(),
        1,
        "only the Discover leaves before the offer"
    );
    assert_eq!(
        discover
            .first()
            .map(|frame| dhcp_proto::xid::extract(dhcp_payload(frame))),
        Some(DHCP_XID)
    );
    assert_eq!(offered, Progress::Worked);
    assert_eq!(acknowledged, Progress::Worked);
    let (level, lease) = lease.expect("DHCP lease observed");
    assert_eq!(level, net_observe::Level::Info);
    assert_eq!(lease.address, ADDRESS);
    assert_eq!(lease.server, DHCP_SERVER);
    assert_eq!(lease.phase, net_observe::interface::DhcpPhase::Bound);
}

#[cfg(feature = "ipv4")]
#[test]
fn socket_error_reaches_the_address_method() {
    // ARRANGE
    let now = Instant::now();
    let wall = SystemTime::UNIX_EPOCH;
    let mut driver: Loop<Mock> = Loop::new(runtime(), config(vec![dhcp_interface()]));
    block_on(boot(&mut driver, now, wall));
    let id = open_socket(
        &driver,
        netmodel::Family::V4,
        ip_method::SocketSlot::Primary,
    );

    // ACT
    block_on(net_runtime::Packet::close(driver.runtime(), id)).expect("close socket");
    let waited = block_on(driver.wait_event()).expect("wait event");
    block_on(settle(&mut driver, now, wall));
    let noticed = driver.drain_observations().any(|observation| {
        matches!(
            observation.event,
            net_observe::Event::Notice { ref text, .. } if text.contains("DHCP acquire failed")
        )
    });

    // ASSERT
    assert_eq!(waited, Progress::Worked);
    assert!(
        noticed,
        "the receive error must reach the DHCP method as a socket error"
    );
    assert!(
        driver.sockets().get(id).is_none(),
        "the dead socket must be released after the receive error"
    );
}

#[cfg(feature = "ipv6")]
fn advertisement() -> Vec<u8> {
    let prefixes = [ndp::types::PrefixInformation {
        prefix: PREFIX_V6,
        prefix_len: 64,
        on_link: true,
        autonomous: true,
        valid_lifetime: 7200,
        preferred_lifetime: 3600,
    }];
    let header = ndp::message::AdvertisementHeader {
        hop_limit: 64,
        managed_flag: false,
        other_flag: false,
        router_lifetime: 1800,
        reachable_time: 0,
        retrans_timer: 0,
    };
    let content = build::router::Advertisement {
        header,
        link_layer: None,
        mtu: None,
        prefixes: &prefixes,
        dns_lifetime: 0,
        dns_servers: &[],
    };
    let mut buffer = [0_u8; 256];

    build::router::advertisement(ROUTER, ROUTER, &content, &mut buffer)
        .expect("advertisement fits")
        .to_vec()
}

#[cfg(feature = "ipv6")]
#[test]
fn slaac_advertisement_provisions_an_address_through_the_socket_path() {
    // ARRANGE
    let now = Instant::now();
    let wall = SystemTime::UNIX_EPOCH;
    let announced = netmodel::Config {
        ipv6: true,
        dns: Vec::new(),
        interfaces: vec![slaac_interface()],
    };
    let mut driver: Loop<Mock> = Loop::new(runtime(), announced);
    block_on(boot(&mut driver, now, wall));
    let id = open_socket(
        &driver,
        netmodel::Family::V6,
        ip_method::SocketSlot::Primary,
    );
    let kind = driver.sockets().get(id).map(|owner| owner.kind);
    let solicited = driver.runtime().sent(id).expect("sent frames");
    let packet = advertisement();

    // ACT
    driver
        .runtime()
        .push_inbound_from(id, &packet, ROUTER)
        .expect("queue advertisement");
    let waited = block_on(driver.wait_event()).expect("wait event");
    block_on(settle(&mut driver, now, wall));
    let update = driver.drain_observations().find_map(|observation| {
        let net_observe::Event::SlaacUpdate { update, .. } = observation.event else {
            return None;
        };
        Some((observation.level, update))
    });

    // ASSERT
    assert_eq!(kind, Some(ip_method::SocketKind::RawIcmpv6));
    assert_eq!(
        solicited.first().map(|frame| frame[0]),
        Some(ndp::ROUTER_SOLICITATION),
        "the opened socket lets SLAAC solicit a router"
    );
    assert_eq!(waited, Progress::Worked);
    let (level, update) = update.expect("SLAAC update observed");
    assert_eq!(level, net_observe::Level::Info);
    assert_eq!(update.routers, vec![ROUTER]);
    assert_eq!(update.addresses.len(), 1);
    assert_eq!(update.addresses[0].1, 64);
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
        netmodel::Family::V4,
        ip_method::SocketSlot::Primary,
    );
    let found_secondary = table.find(
        &name("eth1"),
        netmodel::Family::V4,
        ip_method::SocketSlot::Secondary,
    );
    let missing = table.find(
        &name("eth0"),
        netmodel::Family::V6,
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
