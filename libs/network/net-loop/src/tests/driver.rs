//! End-to-end driver tests over the mock runtime.

use alloc::string::String;
use alloc::vec::Vec;
use core::future::Future;
use core::net::{IpAddr, Ipv4Addr};
use std::io;
use std::time::{Instant, SystemTime};

use net_runtime::mock::Mock;

use super::{block_on, name, runtime};
use crate::{Loop, Pending, Progress};

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
    fn detach(&self, port: &netmodel::Name) -> impl Future<Output = io::Result<()>> + Send {
        self.mock.detach(port)
    }

    fn ensure_bridge(
        &self,
        name: &netmodel::Name,
        stp: bool,
    ) -> impl Future<Output = io::Result<()>> + Send {
        self.mock.ensure_bridge(name, stp)
    }

    fn link(
        &self,
        name: &netmodel::Name,
    ) -> impl Future<Output = io::Result<Option<netmodel::Link>>> + Send {
        self.mock.link(name)
    }

    fn links(&self) -> impl Future<Output = io::Result<Vec<netmodel::Link>>> + Send {
        std::future::ready(Err(io::Error::other("link inventory failed")))
    }

    fn set_down(&self, name: &netmodel::Name) -> impl Future<Output = io::Result<()>> + Send {
        self.mock.set_down(name)
    }

    fn set_master(
        &self,
        port: &netmodel::Name,
        master: &netmodel::Name,
    ) -> impl Future<Output = io::Result<()>> + Send {
        self.mock.set_master(port, master)
    }

    fn set_up(&self, name: &netmodel::Name) -> impl Future<Output = io::Result<()>> + Send {
        self.mock.set_up(name)
    }
}

struct BrokenLink {
    mock: Mock,
}

impl net_runtime::Runtime for BrokenLink {
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

impl net_runtime::Link for BrokenLink {
    fn detach(&self, port: &netmodel::Name) -> impl Future<Output = io::Result<()>> + Send {
        self.mock.detach(port)
    }

    fn ensure_bridge(
        &self,
        name: &netmodel::Name,
        stp: bool,
    ) -> impl Future<Output = io::Result<()>> + Send {
        self.mock.ensure_bridge(name, stp)
    }

    fn link(
        &self,
        _name: &netmodel::Name,
    ) -> impl Future<Output = io::Result<Option<netmodel::Link>>> + Send {
        std::future::ready(Err(io::Error::other("link lookup failed")))
    }

    fn links(&self) -> impl Future<Output = io::Result<Vec<netmodel::Link>>> + Send {
        self.mock.links()
    }

    fn set_down(&self, name: &netmodel::Name) -> impl Future<Output = io::Result<()>> + Send {
        self.mock.set_down(name)
    }

    fn set_master(
        &self,
        port: &netmodel::Name,
        master: &netmodel::Name,
    ) -> impl Future<Output = io::Result<()>> + Send {
        self.mock.set_master(port, master)
    }

    fn set_up(&self, name: &netmodel::Name) -> impl Future<Output = io::Result<()>> + Send {
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
    fn detach(&self, port: &netmodel::Name) -> impl Future<Output = io::Result<()>> + Send {
        self.mock.detach(port)
    }

    fn ensure_bridge(
        &self,
        name: &netmodel::Name,
        stp: bool,
    ) -> impl Future<Output = io::Result<()>> + Send {
        self.mock.ensure_bridge(name, stp)
    }

    fn link(
        &self,
        name: &netmodel::Name,
    ) -> impl Future<Output = io::Result<Option<netmodel::Link>>> + Send {
        self.mock.link(name)
    }

    fn links(&self) -> impl Future<Output = io::Result<Vec<netmodel::Link>>> + Send {
        self.mock.links()
    }

    fn set_down(&self, name: &netmodel::Name) -> impl Future<Output = io::Result<()>> + Send {
        self.mock.set_down(name)
    }

    fn set_master(
        &self,
        port: &netmodel::Name,
        master: &netmodel::Name,
    ) -> impl Future<Output = io::Result<()>> + Send {
        self.mock.set_master(port, master)
    }

    fn set_up(&self, _name: &netmodel::Name) -> impl Future<Output = io::Result<()>> + Send {
        // A slow PHY stays down after the administrative up request; carrier
        // only arrives with the link event.
        std::future::ready(Ok(()))
    }
}

fn ethernet() -> net_supervisor::InterfaceConfig {
    net_supervisor::InterfaceConfig {
        name: String::from("eth0"),
        kind: net_supervisor::InterfaceKind::Ethernet,
        ipv4: Some(address()),
        ipv6: None,
        bridge: None,
    }
}

fn bridge() -> net_supervisor::InterfaceConfig {
    net_supervisor::InterfaceConfig {
        name: String::from("br0"),
        kind: net_supervisor::InterfaceKind::Bridge,
        ipv4: Some(address()),
        ipv6: None,
        bridge: Some(net_supervisor::BridgeConfig {
            port: vec![String::from("auto")],
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

fn config(interfaces: Vec<net_supervisor::InterfaceConfig>) -> net_supervisor::Config {
    net_supervisor::Config {
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
    block_on(net_runtime::Address::list(&driver.runtime, interface)).expect("list addresses")
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
    block_on(net_runtime::Link::link(&driver.runtime, interface))
        .expect("query link")
        .expect("known link")
        .state
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
fn down_link_is_activated_before_the_carrier_probe() {
    // ARRANGE
    let now = Instant::now();
    let wall = SystemTime::UNIX_EPOCH;
    let mock = Mock::new(now, wall);
    mock.set_link(netmodel::Link {
        name: name("eth0"),
        index: 2,
        mac: netmodel::Mac::default(),
        state: netmodel::LinkState::Down,
        is_virtual: false,
        master: None,
    });
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
    assert_eq!(driver.runtime.bridge_stp(&name("br0")), Some(false));
    let names: Vec<netmodel::Name> = driver
        .interfaces()
        .iter()
        .map(|controller| controller.link().name.clone())
        .collect();
    assert_eq!(names, vec![name("eth0"), name("br0")]);
    let port = block_on(net_runtime::Link::link(&driver.runtime, &name("eth0")))
        .expect("query port")
        .expect("known port");
    let master = block_on(net_runtime::Link::link(&driver.runtime, &name("br0")))
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

    // ASSERT
    assert_eq!(started, Progress::Worked);
    assert_eq!(driver.status().state, net_supervisor::State::Degraded);
}

#[test]
fn discovery_refresh_failure_still_feeds_the_supervisor() {
    // ARRANGE
    let now = Instant::now();
    let wall = SystemTime::UNIX_EPOCH;
    let mut driver: Loop<Mock> = Loop::new(Mock::new(now, wall), config(vec![ethernet()]));
    driver.add_interface(ip_method::LinkInfo {
        name: name("eth0"),
        index: 2,
        mac: netmodel::Mac::default(),
        state: netmodel::LinkState::Down,
    });
    drop(driver.interfaces[0].step(
        now,
        wall,
        iface::Input::Command(iface::Command::Configure {
            v4: Some(address()),
            v6: None,
        }),
    ));
    loop {
        if matches!(
            driver.interfaces[0].step(now, wall, iface::Input::Poll),
            iface::Step::Wait
        ) {
            break;
        }
    }
    block_on(driver.start(now));
    driver.pending.clear();
    driver
        .pending
        .push_back(Pending::Links(vec![netmodel::Link {
            name: name("eth0"),
            index: 2,
            mac: netmodel::Mac::default(),
            state: netmodel::LinkState::Up,
            is_virtual: false,
            master: None,
        }]));

    // ACT
    let error = block_on(driver.step(now, wall)).expect_err("refresh effect fails");
    block_on(settle(&mut driver, now, wall));

    // ASSERT
    assert_eq!(error.kind(), io::ErrorKind::NotFound);
    assert_eq!(driver.status().state, net_supervisor::State::Degraded);
}

#[test]
fn degraded_probe_recovers_when_the_link_comes_up_on_retry() {
    // ARRANGE
    let now = Instant::now();
    let wall = SystemTime::UNIX_EPOCH;
    let mock = Mock::new(now, wall);
    mock.set_link(netmodel::Link {
        name: name("eth0"),
        index: 2,
        mac: netmodel::Mac::default(),
        state: netmodel::LinkState::NoCarrier,
        is_virtual: false,
        master: None,
    });
    let mut driver: Loop<Mock> = Loop::new(mock, config(vec![ethernet()]));

    // ACT
    block_on(boot(&mut driver, now, wall));
    let degraded = driver.status().state;
    let retry = driver.next_wake(now).expect("retry deadline");
    driver.runtime.set_link(netmodel::Link {
        name: name("eth0"),
        index: 2,
        mac: netmodel::Mac::default(),
        state: netmodel::LinkState::Up,
        is_virtual: false,
        master: None,
    });
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
    mock.set_link(netmodel::Link {
        name: name("eth0"),
        index: 2,
        mac: netmodel::Mac::default(),
        state: netmodel::LinkState::Down,
        is_virtual: false,
        master: None,
    });
    let mut driver: Loop<SlowPhy> = Loop::new(SlowPhy { mock }, config(vec![ethernet()]));

    // ACT
    block_on(boot(&mut driver, now, wall));
    let degraded = driver.status().state;
    driver.runtime.mock.set_link(netmodel::Link {
        name: name("eth0"),
        index: 2,
        mac: netmodel::Mac::default(),
        state: netmodel::LinkState::Up,
        is_virtual: false,
        master: None,
    });
    driver.runtime.mock.push_event(netmodel::LinkEvent::Up {
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
fn failed_link_application_still_reaches_the_supervisor() {
    // ARRANGE
    let now = Instant::now();
    let wall = SystemTime::UNIX_EPOCH;
    let runtime = BrokenLink { mock: runtime() };
    let mut driver: Loop<BrokenLink> = Loop::new(runtime, config(Vec::new()));
    driver.started = true;
    driver.runtime.mock.push_event(netmodel::LinkEvent::Up {
        name: name("eth0"),
        index: 2,
    });
    block_on(driver.wait_event()).expect("wait event");

    // ACT
    let error = block_on(driver.step(now, wall)).expect_err("link lookup fails");
    block_on(driver.step(now, wall)).expect("supervisor event step");

    // ASSERT
    assert_eq!(error.kind(), io::ErrorKind::Other);
    assert_eq!(driver.status().processed_events, 1);
}

#[test]
fn link_event_reaches_the_controller_and_the_supervisor() {
    // ARRANGE
    let now = Instant::now();
    let wall = SystemTime::UNIX_EPOCH;
    let mut driver: Loop<Mock> = Loop::new(runtime(), config(vec![ethernet()]));
    block_on(boot(&mut driver, now, wall));
    driver.runtime.set_link(netmodel::Link {
        name: name("eth0"),
        index: 2,
        mac: netmodel::Mac::default(),
        state: netmodel::LinkState::Down,
        is_virtual: false,
        master: None,
    });
    driver.runtime.push_event(netmodel::LinkEvent::Down {
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
    driver.runtime.push_event(netmodel::LinkEvent::Deleted {
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
        &driver.runtime,
        &name("br0"),
        assigned(),
    ))
    .expect("remove address");
    driver.runtime.push_event(netmodel::LinkEvent::Up {
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
fn wait_event_handles_one_event() {
    // ARRANGE
    let now = Instant::now();
    let wall = SystemTime::UNIX_EPOCH;
    let mut driver: Loop<Mock> = Loop::new(runtime(), config(Vec::new()));
    block_on(boot(&mut driver, now, wall));
    driver.runtime.push_event(netmodel::LinkEvent::Up {
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
    driver.runtime.set_link(netmodel::Link {
        name: name("eth0"),
        index: 2,
        mac: netmodel::Mac::default(),
        state: netmodel::LinkState::Down,
        is_virtual: false,
        master: None,
    });
    driver.runtime.push_event(netmodel::LinkEvent::Down {
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
    driver.runtime.set_link(netmodel::Link {
        name: name("eth0"),
        index: 2,
        mac: netmodel::Mac::default(),
        state: netmodel::LinkState::Down,
        is_virtual: false,
        master: None,
    });
    driver.runtime.push_event(netmodel::LinkEvent::Down {
        name: name("eth0"),
        index: 2,
    });
    block_on(driver.wait_event()).expect("wait event");

    // ACT
    block_on(settle(&mut driver, now, wall));
    let down = driver.status();
    driver.runtime.set_link(netmodel::Link {
        name: name("eth0"),
        index: 2,
        mac: netmodel::Mac::default(),
        state: netmodel::LinkState::Up,
        is_virtual: false,
        master: None,
    });
    driver.runtime.push_event(netmodel::LinkEvent::Up {
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
    driver.runtime.set_link(netmodel::Link {
        name: name("eth0"),
        index: 2,
        mac: netmodel::Mac::default(),
        state: netmodel::LinkState::NoCarrier,
        is_virtual: false,
        master: None,
    });
    driver.runtime.push_event(netmodel::LinkEvent::Down {
        name: name("eth0"),
        index: 2,
    });
    block_on(driver.wait_event()).expect("wait event");

    // ACT
    block_on(settle(&mut driver, now, wall));
    let lost = driver.status();
    driver.runtime.set_link(netmodel::Link {
        name: name("eth0"),
        index: 2,
        mac: netmodel::Mac::default(),
        state: netmodel::LinkState::Up,
        is_virtual: false,
        master: None,
    });
    driver.runtime.push_event(netmodel::LinkEvent::Up {
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
fn enslaving_a_port_makes_a_down_bridge_carry() {
    // ARRANGE
    let now = Instant::now();
    let wall = SystemTime::UNIX_EPOCH;
    let mut driver: Loop<Mock> = Loop::new(runtime(), config(Vec::new()));
    driver.runtime.set_link(netmodel::Link {
        name: name("br0"),
        index: 9,
        mac: netmodel::Mac::default(),
        state: netmodel::LinkState::Down,
        is_virtual: true,
        master: None,
    });
    driver.add_interface(ip_method::LinkInfo {
        name: name("br0"),
        index: 9,
        mac: netmodel::Mac::default(),
        state: netmodel::LinkState::Down,
    });
    driver.add_interface(ip_method::LinkInfo {
        name: name("eth0"),
        index: 2,
        mac: netmodel::Mac::default(),
        state: netmodel::LinkState::Up,
    });
    driver.started = true;
    driver.pending.push_back(Pending::Enslaved {
        port: name("eth0"),
        master: name("br0"),
    });

    // ACT
    block_on(driver.step(now, wall)).expect("enslave step");
    block_on(driver.step(now, wall)).expect("carrier step");

    // ASSERT
    assert_eq!(
        driver.interfaces()[0].snapshot().link,
        netmodel::LinkState::Up
    );
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
fn next_wake_keeps_a_controller_deadline_when_the_supervisor_has_none() {
    // ARRANGE
    let now = Instant::now();
    let wall = SystemTime::UNIX_EPOCH;
    let mut driver: Loop<Mock> = Loop::new(runtime(), config(Vec::new()));
    driver.started = true;
    driver.add_interface(ip_method::LinkInfo {
        name: name("eth0"),
        index: 2,
        mac: netmodel::Mac::default(),
        state: netmodel::LinkState::Up,
    });
    driver.interfaces[0].step(
        now,
        wall,
        iface::Input::Command(iface::Command::Configure {
            v4: Some(address()),
            v6: None,
        }),
    );

    // ACT
    let wake = driver.next_wake(now);

    // ASSERT
    assert_eq!(wake, Some(now));
}

#[test]
fn discovery_refreshes_a_readded_link() {
    // ARRANGE
    let now = Instant::now();
    let wall = SystemTime::UNIX_EPOCH;
    let mut driver: Loop<Mock> = Loop::new(runtime(), config(vec![ethernet()]));
    block_on(boot(&mut driver, now, wall));
    driver
        .pending
        .push_back(Pending::Links(vec![netmodel::Link {
            name: name("eth0"),
            index: 7,
            mac: netmodel::Mac::default(),
            state: netmodel::LinkState::Down,
            is_virtual: false,
            master: None,
        }]));

    // ACT
    block_on(driver.step(now, wall)).expect("discovery step");

    // ASSERT
    assert_eq!(driver.interfaces()[0].link().index, 7);
    assert_eq!(
        driver.interfaces()[0].snapshot().link,
        netmodel::LinkState::Down
    );
}
