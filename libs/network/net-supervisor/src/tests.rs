//! Supervisor policy tests driving the sans-io state machine.

use core::net::{IpAddr, Ipv4Addr, Ipv6Addr};
use std::io;
use std::time::{Duration, Instant};

use netmodel::{Link, LinkState, Mac, Name};

use crate::{
    BridgeConfig, Command, Config, Effect, EffectKind, Input, InterfaceConfig, InterfaceKind,
    State, Step, Supervisor,
};

#[derive(Debug, Clone, PartialEq, Eq)]
enum Seen {
    Discover,
    Probe,
    SetLinkUp(Name),
    Configure {
        name: Name,
        v4: bool,
        v6: bool,
    },
    Reconcile(Name),
    Bridge {
        name: Name,
        stp: bool,
        ports: Vec<String>,
    },
    Enslave {
        port: Name,
        master: Name,
    },
    Detach(Name),
    Shutdown(Name),
    Publish,
    Resolver {
        v4: Vec<Ipv4Addr>,
        v6: Vec<Ipv6Addr>,
    },
}

fn name(value: &str) -> Name {
    Name::new(value).expect("valid name")
}

fn link(label: &str, index: u32, state: LinkState) -> Link {
    Link {
        name: name(label),
        index,
        mac: Mac::default(),
        admin: state.admin(),
        carrier: state.carrier(),
        is_virtual: false,
        master: None,
    }
}

fn ethernet(label: &str, dhcp: bool) -> InterfaceConfig {
    InterfaceConfig {
        name: label.to_owned(),
        kind: InterfaceKind::Ethernet,
        ipv4: Some(netmodel::Ipv4Config {
            dhcp,
            ..netmodel::Ipv4Config::default()
        }),
        ipv6: None,
        bridge: None,
    }
}

fn bridge() -> InterfaceConfig {
    InterfaceConfig {
        name: "br0".to_owned(),
        kind: InterfaceKind::Bridge,
        ipv4: Some(netmodel::Ipv4Config {
            addresses: vec![(Ipv4Addr::new(10, 0, 0, 2), 24)],
            ..netmodel::Ipv4Config::default()
        }),
        ipv6: None,
        bridge: Some(BridgeConfig {
            port: vec!["auto".to_owned()],
            stp: false,
        }),
    }
}

fn config(interfaces: Vec<InterfaceConfig>) -> Config {
    Config {
        ipv6: true,
        dns: vec![IpAddr::V4(Ipv4Addr::new(9, 9, 9, 9))],
        interfaces,
    }
}

fn configured(link_state: LinkState) -> iface::Snapshot {
    iface::Snapshot {
        link: link_state,
        v4: Some(iface::V4View::Static(ip_static::View {
            address: Ipv4Addr::new(10, 0, 0, 2),
            prefix_len: 24,
            gateway: None,
            dns: Vec::new(),
        })),
        v4_state: ip_method::State::Bound,
        v6: None,
        v6_state: ip_method::State::Stopped,
        notices: Vec::new(),
    }
}

fn failed() -> iface::Snapshot {
    iface::Snapshot {
        link: LinkState::Down,
        v4: None,
        v4_state: ip_method::State::Degraded,
        v6: None,
        v6_state: ip_method::State::Stopped,
        notices: Vec::new(),
    }
}

fn describe(effect: Effect<'_>) -> Seen {
    match effect {
        Effect::Discover => Seen::Discover,
        Effect::ProbeCarriers => Seen::Probe,
        Effect::SetLinkUp { name } => Seen::SetLinkUp(name),
        Effect::ConfigureInterface { name, v4, v6 } => Seen::Configure {
            name,
            v4: v4.is_some(),
            v6: v6.is_some(),
        },
        Effect::ReconcileInterface { name } => Seen::Reconcile(name),
        Effect::EnsureBridge { name, config } => Seen::Bridge {
            name,
            stp: config.stp,
            ports: config.port.clone(),
        },
        Effect::Enslave { port, master } => Seen::Enslave { port, master },
        Effect::Detach { port } => Seen::Detach(port),
        Effect::ShutdownInterface { name } => Seen::Shutdown(name),
        Effect::Publish => Seen::Publish,
        Effect::WriteResolver { v4, v6 } => Seen::Resolver {
            v4: v4.to_vec(),
            v6: v6.to_vec(),
        },
    }
}

fn consume(supervisor: &mut Supervisor, now: Instant, input: Input<'_>) -> Option<Seen> {
    match supervisor.step(now, input) {
        Step::Effect(effect) => Some(describe(effect)),
        Step::Wait => None,
    }
}

fn run(supervisor: &mut Supervisor, now: Instant, input: Input<'_>) -> Vec<Seen> {
    let mut seen = Vec::new();
    if let Some(effect) = consume(supervisor, now, input) {
        seen.push(effect);
    }
    while let Some(effect) = consume(supervisor, now, Input::Poll) {
        seen.push(effect);
    }

    seen
}

fn boot(interfaces: Vec<InterfaceConfig>, links: &[Link], probe: &[Link]) -> Supervisor {
    let mut supervisor = Supervisor::new(config(interfaces));
    let now = Instant::now();
    run(&mut supervisor, now, Input::Command(Command::Initialize));
    run(&mut supervisor, now, Input::Links(links));
    run(&mut supervisor, now, Input::Carriers(probe));

    supervisor
}

#[test]
fn new_starts_uninitialized_and_idles() {
    // ARRANGE
    let supervisor = Supervisor::new(Config::default());
    let now = Instant::now();

    // ACT
    let status = supervisor.status();

    // ASSERT
    assert_eq!(status.state, State::Uninitialized);
    assert!(supervisor.next_wake(now).is_none());
}

#[test]
fn valid_transitions_advance_state() {
    // ARRANGE
    let pairs = [
        (State::Uninitialized, State::Initializing),
        (State::Initializing, State::Operational),
        (State::Initializing, State::Degraded),
        (State::Operational, State::Ready),
        (State::Operational, State::Degraded),
        (State::Ready, State::Degraded),
        (State::Degraded, State::Initializing),
        (State::Degraded, State::Operational),
    ];
    for (from, to) in pairs {
        let mut snapshot = crate::state::Snapshot {
            state: from,
            ..crate::state::Snapshot::default()
        };

        // ACT
        let moved = snapshot.transition(to);

        // ASSERT
        assert!(moved, "{from:?} -> {to:?} must be valid");
        assert_eq!(snapshot.state, to);
    }
}

#[test]
fn invalid_transitions_leave_state_unchanged() {
    // ARRANGE
    let pairs = [
        (State::Uninitialized, State::Operational),
        (State::Uninitialized, State::Ready),
        (State::Uninitialized, State::Degraded),
        (State::Initializing, State::Ready),
        (State::Operational, State::Initializing),
        (State::Ready, State::Operational),
        (State::Degraded, State::Ready),
    ];
    for (from, to) in pairs {
        let mut snapshot = crate::state::Snapshot {
            state: from,
            ..crate::state::Snapshot::default()
        };

        // ACT
        let moved = snapshot.transition(to);

        // ASSERT
        assert!(!moved, "{from:?} -> {to:?} must be invalid");
        assert_eq!(snapshot.state, from);
    }
}

#[test]
fn initialize_discovers_then_probes_then_provisions() {
    // ARRANGE
    let links = [link("eth0", 2, LinkState::Up)];
    let mut supervisor = Supervisor::new(config(vec![bridge(), ethernet("auto", true)]));
    let now = Instant::now();

    // ACT
    let discover = run(&mut supervisor, now, Input::Command(Command::Initialize));
    let probe = run(&mut supervisor, now, Input::Links(&links));
    let provision = run(&mut supervisor, now, Input::Carriers(&links));

    // ASSERT
    assert_eq!(discover.first(), Some(&Seen::Discover));
    assert_eq!(probe, vec![Seen::Probe]);
    assert_eq!(supervisor.status().primary, Some(name("eth0")));
    assert_eq!(supervisor.status().state, State::Ready);
    assert_eq!(supervisor.status().owner, Some(name("eth0")));
    assert_eq!(
        provision.first(),
        Some(&Seen::Resolver {
            v4: vec![Ipv4Addr::new(9, 9, 9, 9)],
            v6: Vec::new(),
        })
    );
    assert!(provision.contains(&Seen::Bridge {
        name: name("br0"),
        stp: false,
        ports: vec!["auto".to_owned()],
    }));
    assert!(provision.contains(&Seen::Enslave {
        port: name("eth0"),
        master: name("br0"),
    }));
    assert!(!provision.iter().any(
        |seen| matches!(seen, Seen::Configure { name: target, .. } if target == &name("br0"))
    ));
    assert!(provision.contains(&Seen::Publish));

    // ACT
    let confirmed = run(
        &mut supervisor,
        now,
        Input::Enslaved {
            port: name("eth0"),
            master: name("br0"),
        },
    );

    // ASSERT
    assert_eq!(supervisor.status().owner, Some(name("br0")));
    assert!(confirmed.iter().any(
        |seen| matches!(seen, Seen::Configure { name: target, .. } if target == &name("br0"))
    ));
}

#[test]
fn missing_link_in_carrier_probe_degrades_without_a_primary() {
    // ARRANGE
    let mut supervisor = Supervisor::new(config(vec![ethernet("auto", true)]));
    let now = Instant::now();
    run(&mut supervisor, now, Input::Command(Command::Initialize));
    run(
        &mut supervisor,
        now,
        Input::Links(&[link("eth0", 2, LinkState::Up)]),
    );

    // ACT
    run(&mut supervisor, now, Input::Carriers(&[]));

    // ASSERT
    assert_eq!(supervisor.status().state, State::Degraded);
    assert_eq!(supervisor.status().primary, None);
}

#[test]
fn no_physical_links_degrades() {
    // ARRANGE
    let virtual_link = Link {
        is_virtual: true,
        ..link("veth0", 4, LinkState::Up)
    };
    let mut supervisor = Supervisor::new(config(Vec::new()));
    let now = Instant::now();
    run(&mut supervisor, now, Input::Command(Command::Initialize));

    // ACT
    let seen = run(&mut supervisor, now, Input::Links(&[virtual_link]));

    // ASSERT
    assert_eq!(supervisor.status().state, State::Degraded);
    assert!(
        !seen
            .iter()
            .any(|effect| matches!(effect, Seen::Probe | Seen::SetLinkUp(_)))
    );
}

#[test]
fn down_links_are_activated_before_the_carrier_probe() {
    // ARRANGE
    let links = [
        link("eth0", 2, LinkState::Down),
        link("eth1", 3, LinkState::Down),
    ];
    let mut supervisor = Supervisor::new(config(vec![ethernet("auto", true)]));
    let now = Instant::now();
    run(&mut supervisor, now, Input::Command(Command::Initialize));

    // ACT
    let seen = run(&mut supervisor, now, Input::Links(&links));
    let provisioned = run(
        &mut supervisor,
        now,
        Input::Carriers(&[
            link("eth0", 2, LinkState::Up),
            link("eth1", 3, LinkState::Up),
        ]),
    );

    // ASSERT
    assert_eq!(
        seen,
        vec![
            Seen::SetLinkUp(name("eth0")),
            Seen::SetLinkUp(name("eth1")),
            Seen::Probe,
        ]
    );
    assert_eq!(supervisor.status().state, State::Ready);
    assert_eq!(supervisor.status().primary, Some(name("eth0")));
    assert!(provisioned.contains(&Seen::Configure {
        name: name("eth0"),
        v4: true,
        v6: false,
    }));
}

#[test]
fn degraded_link_event_up_requeues_discovery() {
    // ARRANGE
    let links = [link("eth0", 2, LinkState::Down)];
    let mut supervisor = Supervisor::new(config(vec![ethernet("auto", true)]));
    let now = Instant::now();
    run(&mut supervisor, now, Input::Command(Command::Initialize));
    run(&mut supervisor, now, Input::Links(&links));
    let degraded = run(
        &mut supervisor,
        now,
        Input::Carriers(&[link("eth0", 2, LinkState::NoCarrier)]),
    );

    // ACT
    let seen = run(
        &mut supervisor,
        now,
        Input::LinkEvent(netmodel::LinkEvent::Up {
            name: name("eth0"),
            index: 2,
        }),
    );

    // ASSERT
    assert_eq!(supervisor.status().state, State::Degraded);
    assert!(supervisor.status().primary.is_none());
    assert!(!degraded.iter().any(|effect| matches!(
        effect,
        Seen::Configure { .. } | Seen::Bridge { .. } | Seen::Enslave { .. }
    )));
    assert_eq!(seen.first(), Some(&Seen::Discover));
}

#[test]
fn set_link_up_failure_degrades_and_retries() {
    // ARRANGE
    let mut supervisor = Supervisor::new(config(Vec::new()));
    let now = Instant::now();
    run(&mut supervisor, now, Input::Command(Command::Initialize));

    // ACT
    run(
        &mut supervisor,
        now,
        Input::EffectFailed {
            kind: EffectKind::SetLinkUp,
            error: io::Error::other("boom"),
        },
    );

    // ASSERT
    assert_eq!(supervisor.status().state, State::Degraded);
    let retry = supervisor.next_wake(now).expect("retry scheduled");
    assert_eq!(
        retry,
        now.checked_add(Duration::from_secs(1))
            .expect("retry deadline")
    );
    assert!(
        retry
            < now
                .checked_add(Duration::from_secs(30))
                .expect("reconcile deadline")
    );
}

#[test]
fn no_carrier_degrades_without_provisioning() {
    // ARRANGE
    let links = [link("eth0", 2, LinkState::NoCarrier)];
    let mut supervisor = Supervisor::new(config(vec![bridge(), ethernet("auto", true)]));
    let now = Instant::now();
    run(&mut supervisor, now, Input::Command(Command::Initialize));
    let inventory = run(&mut supervisor, now, Input::Links(&links));

    // ACT
    let seen = run(
        &mut supervisor,
        now,
        Input::Carriers(&[link("eth0", 2, LinkState::NoCarrier)]),
    );

    // ASSERT
    assert_eq!(inventory, vec![Seen::Probe]);
    assert_eq!(supervisor.status().state, State::Degraded);
    assert!(!seen.iter().any(|effect| matches!(
        effect,
        Seen::Bridge { .. } | Seen::Enslave { .. } | Seen::Configure { .. }
    )));
}

#[test]
fn admin_down_carriers_degrade_without_provisioning() {
    // ARRANGE
    let links = [link("eth0", 2, LinkState::Down)];
    let mut supervisor = Supervisor::new(config(vec![bridge(), ethernet("auto", true)]));
    let now = Instant::now();
    run(&mut supervisor, now, Input::Command(Command::Initialize));
    run(&mut supervisor, now, Input::Links(&links));

    // ACT
    let seen = run(
        &mut supervisor,
        now,
        Input::Carriers(&[link("eth0", 2, LinkState::Down)]),
    );

    // ASSERT
    assert_eq!(supervisor.status().state, State::Degraded);
    assert!(!seen.iter().any(|effect| matches!(
        effect,
        Seen::Bridge { .. } | Seen::Enslave { .. } | Seen::Configure { .. }
    )));
}

#[test]
fn dormant_carriers_degrade_without_provisioning() {
    // ARRANGE
    let links = [link("eth0", 2, LinkState::Dormant)];
    let mut supervisor = Supervisor::new(config(vec![bridge(), ethernet("auto", true)]));
    let now = Instant::now();
    run(&mut supervisor, now, Input::Command(Command::Initialize));
    run(&mut supervisor, now, Input::Links(&links));

    // ACT
    let seen = run(
        &mut supervisor,
        now,
        Input::Carriers(&[link("eth0", 2, LinkState::Dormant)]),
    );

    // ASSERT
    assert_eq!(supervisor.status().state, State::Degraded);
    assert!(!seen.iter().any(|effect| matches!(
        effect,
        Seen::Bridge { .. } | Seen::Enslave { .. } | Seen::Configure { .. }
    )));
}

#[test]
fn carrier_probe_updates_admin_state_from_the_fresh_link() {
    // ARRANGE
    let links = [link("eth0", 2, LinkState::Down)];
    let mut supervisor = Supervisor::new(config(vec![ethernet("auto", true)]));
    let now = Instant::now();
    run(&mut supervisor, now, Input::Command(Command::Initialize));
    run(&mut supervisor, now, Input::Links(&links));

    // ACT
    run(
        &mut supervisor,
        now,
        Input::Carriers(&[link("eth0", 2, LinkState::Up)]),
    );

    // ASSERT
    assert_eq!(supervisor.links[0].admin, netmodel::AdminState::Up);
    assert_eq!(supervisor.links[0].carrier, netmodel::CarrierState::Up);
    assert_eq!(supervisor.status().state, State::Ready);
}

#[test]
fn ethernet_addresses_go_to_the_bridge_owner() {
    // ARRANGE
    let links = [link("eth0", 2, LinkState::Up)];
    let mut supervisor = Supervisor::new(config(vec![bridge(), ethernet("auto", true)]));
    let now = Instant::now();
    run(&mut supervisor, now, Input::Command(Command::Initialize));
    run(&mut supervisor, now, Input::Links(&links));

    // ACT
    run(&mut supervisor, now, Input::Carriers(&links));
    let seen = run(
        &mut supervisor,
        now,
        Input::Enslaved {
            port: name("eth0"),
            master: name("br0"),
        },
    );

    // ASSERT
    assert!(seen.contains(&Seen::Configure {
        name: name("br0"),
        v4: true,
        v6: false,
    }));
}

#[test]
fn active_bridge_owner_reconciles_on_enslavement() {
    // ARRANGE
    let links = [link("eth0", 2, LinkState::Up)];
    let mut supervisor = boot(vec![bridge(), ethernet("auto", true)], &links, &links);
    let now = Instant::now();
    let first = run(
        &mut supervisor,
        now,
        Input::Enslaved {
            port: name("eth0"),
            master: name("br0"),
        },
    );
    run(
        &mut supervisor,
        now,
        Input::InterfaceSnapshot {
            name: name("br0"),
            snapshot: &configured(LinkState::Up),
        },
    );

    // ACT
    let second = run(
        &mut supervisor,
        now,
        Input::Enslaved {
            port: name("eth0"),
            master: name("br0"),
        },
    );

    // ASSERT
    assert!(first.contains(&Seen::Configure {
        name: name("br0"),
        v4: true,
        v6: false,
    }));
    assert!(second.contains(&Seen::Reconcile(name("br0"))));
    assert!(!second.iter().any(
        |effect| matches!(effect, Seen::Configure { name: target, .. } if target == &name("br0"))
    ));
}

#[test]
fn primary_loss_promotes_a_configured_backup() {
    // ARRANGE
    let links = [
        link("eth0", 2, LinkState::Up),
        link("eth1", 3, LinkState::Up),
    ];
    let mut supervisor = boot(vec![ethernet("auto", true)], &links, &links);
    let now = Instant::now();
    run(
        &mut supervisor,
        now,
        Input::InterfaceSnapshot {
            name: name("eth0"),
            snapshot: &configured(LinkState::Up),
        },
    );

    // ACT
    let seen = run(
        &mut supervisor,
        now,
        Input::LinkEvent(netmodel::LinkEvent::Down {
            name: name("eth0"),
            index: 2,
        }),
    );

    // ASSERT
    assert_eq!(supervisor.status().primary, Some(name("eth1")));
    assert_eq!(supervisor.status().backups, vec![name("eth0")]);
    assert_eq!(supervisor.status().state, State::Operational);
    assert!(seen.contains(&Seen::Configure {
        name: name("eth1"),
        v4: true,
        v6: false,
    }));
}

#[test]
fn primary_loss_without_a_backup_degrades() {
    // ARRANGE
    let links = [link("eth0", 2, LinkState::Up)];
    let mut supervisor = boot(vec![ethernet("auto", true)], &links, &links);
    let now = Instant::now();

    // ACT
    run(
        &mut supervisor,
        now,
        Input::LinkEvent(netmodel::LinkEvent::Down {
            name: name("eth0"),
            index: 2,
        }),
    );

    // ASSERT
    assert_eq!(supervisor.status().state, State::Degraded);
    assert_eq!(supervisor.status().primary, Some(name("eth0")));
}

#[test]
fn enslaved_owner_failure_rebridges_to_the_backup() {
    // ARRANGE
    let links = [
        link("eth0", 2, LinkState::Up),
        link("eth1", 3, LinkState::Up),
    ];
    let mut supervisor = boot(vec![bridge(), ethernet("auto", true)], &links, &links);
    let now = Instant::now();
    run(
        &mut supervisor,
        now,
        Input::Enslaved {
            port: name("eth0"),
            master: name("br0"),
        },
    );
    run(
        &mut supervisor,
        now,
        Input::InterfaceSnapshot {
            name: name("br0"),
            snapshot: &configured(LinkState::Up),
        },
    );

    // ACT
    let seen = run(
        &mut supervisor,
        now,
        Input::InterfaceSnapshot {
            name: name("br0"),
            snapshot: &failed(),
        },
    );

    // ASSERT
    assert_eq!(supervisor.status().primary, Some(name("eth1")));
    assert!(seen.contains(&Seen::Enslave {
        port: name("eth1"),
        master: name("br0"),
    }));
    assert!(seen.contains(&Seen::Detach(name("eth0"))));
}

#[test]
fn owner_no_carrier_fails_over_to_the_backup() {
    // ARRANGE
    let links = [
        link("eth0", 2, LinkState::Up),
        link("eth1", 3, LinkState::Up),
    ];
    let mut supervisor = boot(vec![bridge(), ethernet("auto", true)], &links, &links);
    let now = Instant::now();
    run(
        &mut supervisor,
        now,
        Input::Enslaved {
            port: name("eth0"),
            master: name("br0"),
        },
    );
    run(
        &mut supervisor,
        now,
        Input::InterfaceSnapshot {
            name: name("br0"),
            snapshot: &configured(LinkState::Up),
        },
    );

    // ACT
    let seen = run(
        &mut supervisor,
        now,
        Input::InterfaceSnapshot {
            name: name("br0"),
            snapshot: &configured(LinkState::NoCarrier),
        },
    );

    // ASSERT
    assert_eq!(supervisor.status().primary, Some(name("eth1")));
    assert!(seen.contains(&Seen::Enslave {
        port: name("eth1"),
        master: name("br0"),
    }));
    assert!(seen.contains(&Seen::Detach(name("eth0"))));
}

#[test]
fn primary_removal_promotes_the_first_eligible_backup() {
    // ARRANGE
    let links = [
        link("eth0", 2, LinkState::Up),
        link("eth1", 3, LinkState::Up),
    ];
    let mut supervisor = boot(vec![ethernet("auto", true)], &links, &links);
    let now = Instant::now();

    // ACT
    run(
        &mut supervisor,
        now,
        Input::LinkEvent(netmodel::LinkEvent::Deleted {
            name: name("eth0"),
            index: 2,
        }),
    );

    // ASSERT
    assert_eq!(supervisor.status().primary, Some(name("eth1")));
    assert_eq!(supervisor.status().backups.len(), 0);
}

#[test]
fn backup_recovery_restores_the_primary() {
    // ARRANGE
    let links = [
        link("eth0", 2, LinkState::Up),
        link("eth1", 3, LinkState::Up),
    ];
    let mut supervisor = boot(vec![ethernet("auto", true)], &links, &links);
    let now = Instant::now();
    run(
        &mut supervisor,
        now,
        Input::LinkEvent(netmodel::LinkEvent::Down {
            name: name("eth0"),
            index: 2,
        }),
    );
    assert_eq!(supervisor.status().primary, Some(name("eth1")));

    // ACT
    run(
        &mut supervisor,
        now,
        Input::LinkEvent(netmodel::LinkEvent::Up {
            name: name("eth0"),
            index: 2,
        }),
    );

    // ASSERT
    assert_eq!(supervisor.status().primary, Some(name("eth0")));
    assert_eq!(supervisor.status().backups, vec![name("eth1")]);
}

#[test]
fn configured_owner_recovers_operational() {
    // ARRANGE
    let links = [link("eth0", 2, LinkState::Up)];
    let mut supervisor = boot(vec![ethernet("auto", true)], &links, &links);
    let now = Instant::now();
    run(
        &mut supervisor,
        now,
        Input::LinkEvent(netmodel::LinkEvent::Down {
            name: name("eth0"),
            index: 2,
        }),
    );
    assert_eq!(supervisor.status().state, State::Degraded);

    // ACT
    run(
        &mut supervisor,
        now,
        Input::InterfaceSnapshot {
            name: name("eth0"),
            snapshot: &configured(LinkState::Up),
        },
    );

    // ASSERT
    assert_eq!(supervisor.status().state, State::Operational);
}

#[test]
fn timer_reconciles_only_when_due() {
    // ARRANGE
    let links = [link("eth0", 2, LinkState::Up)];
    let mut supervisor = boot(vec![bridge(), ethernet("auto", true)], &links, &links);
    let now = Instant::now();
    let due = now + Duration::from_secs(30);
    run(
        &mut supervisor,
        now,
        Input::Enslaved {
            port: name("eth0"),
            master: name("br0"),
        },
    );
    run(
        &mut supervisor,
        now,
        Input::InterfaceSnapshot {
            name: name("br0"),
            snapshot: &configured(LinkState::Up),
        },
    );

    // ACT
    let before_due = run(&mut supervisor, now, Input::Timer);
    let due_effects = run(&mut supervisor, due, Input::Timer);

    // ASSERT
    assert_eq!(
        before_due.len(),
        0,
        "timer before the deadline must be ignored"
    );
    assert!(due_effects.contains(&Seen::Bridge {
        name: name("br0"),
        stp: false,
        ports: vec!["auto".to_owned()],
    }));
    assert!(due_effects.contains(&Seen::Reconcile(name("br0"))));
}

#[test]
fn reconcile_configures_an_owner_that_lost_its_methods() {
    // ARRANGE
    let links = [link("eth0", 2, LinkState::Up)];
    let mut supervisor = boot(vec![ethernet("auto", true)], &links, &links);
    let now = Instant::now();

    // ACT
    let seen = run(&mut supervisor, now, Input::Command(Command::Reconcile));

    // ASSERT
    assert!(seen.contains(&Seen::Configure {
        name: name("eth0"),
        v4: true,
        v6: false,
    }));
}

#[cfg(any(feature = "ipv4", feature = "ipv6"))]
#[test]
fn configure_renders_only_enabled_families() {
    // ARRANGE
    let dual = InterfaceConfig {
        name: "auto".to_owned(),
        kind: InterfaceKind::Ethernet,
        ipv4: Some(netmodel::Ipv4Config {
            dhcp: true,
            ..netmodel::Ipv4Config::default()
        }),
        ipv6: Some(netmodel::Ipv6Config {
            autoconf: true,
            ..netmodel::Ipv6Config::default()
        }),
        bridge: None,
    };
    let links = [link("eth0", 2, LinkState::Up)];
    let mut supervisor = boot(vec![dual], &links, &links);
    let now = Instant::now();

    // ACT
    let seen = run(&mut supervisor, now, Input::Command(Command::Reconcile));

    // ASSERT
    let families = seen.iter().find_map(|effect| match *effect {
        Seen::Configure { v4, v6, .. } => Some((v4, v6)),
        Seen::Discover
        | Seen::Probe
        | Seen::SetLinkUp(_)
        | Seen::Reconcile(_)
        | Seen::Bridge { .. }
        | Seen::Enslave { .. }
        | Seen::Detach(_)
        | Seen::Shutdown(_)
        | Seen::Publish
        | Seen::Resolver { .. } => None,
    });
    assert_eq!(
        families,
        Some((cfg!(feature = "ipv4"), cfg!(feature = "ipv6")))
    );
}

#[cfg(feature = "ipv4")]
#[test]
fn dns_aggregates_static_and_dhcp_views() {
    // ARRANGE
    let mut supervisor = Supervisor::new(config(Vec::new()));
    let owner = name("eth0");
    supervisor.state.primary = Some(owner.clone());
    supervisor.known.insert(owner.clone());
    supervisor.snapshots.insert(
        owner,
        iface::Snapshot {
            link: LinkState::Up,
            v4: Some(iface::V4View::Dhcp(dhcp::View {
                lease: dhcp::Lease {
                    obtained_at: std::time::SystemTime::UNIX_EPOCH,
                    lease_time: Duration::from_secs(60),
                    renewal_time: Duration::from_secs(30),
                    rebind_time: Duration::from_secs(50),
                    server_ip: Ipv4Addr::new(10, 0, 0, 1),
                    assigned_ip: Ipv4Addr::new(10, 0, 0, 2),
                    prefix_len: 24,
                    gateway: None,
                    dns_servers: vec![Ipv4Addr::new(1, 1, 1, 1), Ipv4Addr::new(1, 1, 1, 1)],
                },
                phase: dhcp::Phase::Bound,
            })),
            v4_state: ip_method::State::Bound,
            v6: None,
            v6_state: ip_method::State::Stopped,
            notices: Vec::new(),
        },
    );

    // ACT
    crate::dns::flush(&mut supervisor);
    let seen = run(&mut supervisor, Instant::now(), Input::Poll);

    // ASSERT
    assert_eq!(
        seen.first(),
        Some(&Seen::Resolver {
            v4: vec![Ipv4Addr::new(1, 1, 1, 1)],
            v6: Vec::new(),
        })
    );
}

#[cfg(feature = "ipv6")]
#[test]
fn dns_aggregates_slaac_and_falls_back_to_config() {
    // ARRANGE
    let mut supervisor = Supervisor::new(config(Vec::new()));
    let owner = name("eth0");
    supervisor.state.primary = Some(owner.clone());
    supervisor.known.insert(owner.clone());
    supervisor.snapshots.insert(
        owner,
        iface::Snapshot {
            link: LinkState::Up,
            v4: None,
            v4_state: ip_method::State::Stopped,
            v6: Some(iface::V6View::Slaac(slaac::View {
                addresses: Vec::new(),
                routers: Vec::new(),
                dns: Vec::new(),
            })),
            v6_state: ip_method::State::Bound,
            notices: Vec::new(),
        },
    );

    // ACT
    crate::dns::flush(&mut supervisor);
    let seen = run(&mut supervisor, Instant::now(), Input::Poll);

    // ASSERT
    assert_eq!(
        seen.first(),
        Some(&Seen::Resolver {
            v4: vec![Ipv4Addr::new(9, 9, 9, 9)],
            v6: Vec::new(),
        })
    );
}

#[test]
fn dns_is_unchanged_between_flushes() {
    // ARRANGE
    let mut supervisor = Supervisor::new(config(Vec::new()));
    let owner = name("eth0");
    supervisor.state.primary = Some(owner.clone());
    supervisor.known.insert(owner);

    // ACT
    crate::dns::flush(&mut supervisor);
    let first = run(&mut supervisor, Instant::now(), Input::Poll);
    crate::dns::flush(&mut supervisor);
    let second = run(&mut supervisor, Instant::now(), Input::Poll);

    // ASSERT
    let resolvers = |seen: &[Seen]| {
        seen.iter()
            .filter(|effect| matches!(effect, Seen::Resolver { .. }))
            .count()
    };
    assert_eq!(resolvers(&first), 1);
    assert_eq!(resolvers(&second), 0);
}

#[test]
fn shutdown_queues_every_known_interface_and_resets() {
    // ARRANGE
    let links = [
        link("eth0", 2, LinkState::Up),
        link("eth1", 3, LinkState::Up),
    ];
    let mut supervisor = boot(vec![bridge(), ethernet("auto", true)], &links, &links);
    let now = Instant::now();
    run(
        &mut supervisor,
        now,
        Input::Carriers(&[
            link("eth0", 2, LinkState::Up),
            link("eth1", 3, LinkState::Up),
        ]),
    );

    // ACT
    let seen = run(&mut supervisor, now, Input::Command(Command::Shutdown));

    // ASSERT
    assert!(seen.contains(&Seen::Shutdown(name("eth0"))));
    assert!(seen.contains(&Seen::Shutdown(name("eth1"))));
    assert_eq!(supervisor.status().state, State::Uninitialized);
    assert!(supervisor.status().primary.is_none());
    assert!(supervisor.next_wake(now).is_none());
}

#[test]
fn effect_failure_degrades_during_initialization() {
    // ARRANGE
    let mut supervisor = Supervisor::new(config(Vec::new()));
    let now = Instant::now();
    run(&mut supervisor, now, Input::Command(Command::Initialize));

    // ACT
    run(
        &mut supervisor,
        now,
        Input::EffectFailed {
            kind: EffectKind::Discover,
            error: io::Error::other("boom"),
        },
    );

    // ASSERT
    assert_eq!(supervisor.status().state, State::Degraded);
}

#[test]
fn effect_failure_keeps_running_when_provisioned() {
    // ARRANGE
    let links = [link("eth0", 2, LinkState::Up)];
    let mut supervisor = boot(vec![ethernet("auto", true)], &links, &links);
    let now = Instant::now();

    // ACT
    run(
        &mut supervisor,
        now,
        Input::EffectFailed {
            kind: EffectKind::ConfigureInterface,
            error: io::Error::other("boom"),
        },
    );

    // ASSERT
    assert_eq!(supervisor.status().state, State::Ready);
}

#[test]
fn link_events_are_counted() {
    // ARRANGE
    let mut supervisor = Supervisor::new(config(Vec::new()));
    let now = Instant::now();

    // ACT
    run(
        &mut supervisor,
        now,
        Input::LinkEvent(netmodel::LinkEvent::Error(netmodel::LinkFailure::Closed)),
    );

    // ASSERT
    assert_eq!(supervisor.status().processed_events, 1);
}

#[test]
fn enslaved_and_detached_inputs_track_ownership() {
    // ARRANGE
    let mut supervisor = Supervisor::new(config(Vec::new()));
    let now = Instant::now();

    // ACT
    run(
        &mut supervisor,
        now,
        Input::Enslaved {
            port: name("eth0"),
            master: name("br0"),
        },
    );
    let enslaved_owner = supervisor.status().owner;
    run(&mut supervisor, now, Input::Detached { port: name("eth0") });

    // ASSERT
    assert_eq!(enslaved_owner, Some(name("br0")));
    assert!(!crate::owner::is_enslaved(&supervisor, &name("eth0")));
}

#[test]
fn bridge_ready_registers_the_bridge() {
    // ARRANGE
    let mut supervisor = Supervisor::new(config(Vec::new()));
    let now = Instant::now();

    // ACT
    run(
        &mut supervisor,
        now,
        Input::BridgeReady {
            name: name("br0"),
            index: 5,
            mac: Mac::default(),
            state: LinkState::Up,
        },
    );

    // ASSERT
    assert_eq!(supervisor.status().owner, Some(name("br0")));
}

#[test]
fn added_link_is_queued_for_discovery() {
    // ARRANGE
    let mut supervisor = Supervisor::new(config(Vec::new()));
    let now = Instant::now();

    // ACT
    let seen = run(
        &mut supervisor,
        now,
        Input::LinkEvent(netmodel::LinkEvent::Added {
            name: name("eth0"),
            index: 2,
            mac: Mac::default(),
        }),
    );

    // ASSERT
    assert_eq!(seen.first(), Some(&Seen::Discover));
    assert_eq!(supervisor.status().primary, None);
    assert!(!supervisor.physical.contains(&name("eth0")));
}

#[test]
fn next_wake_reports_queue_and_reconcile_deadline() {
    // ARRANGE
    let mut supervisor = Supervisor::new(config(Vec::new()));
    let now = Instant::now();

    // ACT
    let idle = supervisor.next_wake(now);
    let first = consume(&mut supervisor, now, Input::Command(Command::Initialize));
    let queued = supervisor.next_wake(now);
    run(&mut supervisor, now, Input::Poll);
    let deadline = supervisor.next_wake(now);

    // ASSERT
    assert!(idle.is_none());
    assert_eq!(first, Some(Seen::Discover));
    assert_eq!(queued, Some(now));
    assert_eq!(deadline, now.checked_add(Duration::from_secs(30)));
}

#[test]
fn loopback_is_not_elected_and_no_carrier_degrades() {
    // ARRANGE
    let links = [
        link("lo", 1, LinkState::Up),
        link("eth0", 2, LinkState::NoCarrier),
    ];
    let mut supervisor = Supervisor::new(config(vec![ethernet("auto", true)]));
    let now = Instant::now();
    run(&mut supervisor, now, Input::Command(Command::Initialize));
    run(&mut supervisor, now, Input::Links(&links));

    // ACT
    let seen = run(
        &mut supervisor,
        now,
        Input::Carriers(&[
            link("lo", 1, LinkState::Up),
            link("eth0", 2, LinkState::NoCarrier),
        ]),
    );

    // ASSERT
    assert_eq!(supervisor.status().state, State::Degraded);
    assert_ne!(supervisor.status().primary, Some(name("lo")));
    assert!(!supervisor.known.contains(&name("lo")));
    assert!(
        !seen
            .iter()
            .any(|effect| matches!(effect, Seen::Configure { .. }))
    );
}

#[test]
fn existing_bridge_inventory_reaches_ready_after_feedback() {
    // ARRANGE
    let links = [
        Link {
            is_virtual: true,
            ..link("br0", 5, LinkState::Up)
        },
        Link {
            master: Some(5),
            ..link("eth0", 2, LinkState::Up)
        },
    ];
    let mut supervisor = Supervisor::new(config(vec![bridge(), ethernet("auto", true)]));
    let now = Instant::now();
    run(&mut supervisor, now, Input::Command(Command::Initialize));
    run(&mut supervisor, now, Input::Links(&links));
    run(&mut supervisor, now, Input::Carriers(&links));
    assert_eq!(supervisor.status().state, State::Ready);
    assert_eq!(supervisor.status().primary, Some(name("eth0")));

    // ACT
    let seen = run(
        &mut supervisor,
        now,
        Input::Enslaved {
            port: name("eth0"),
            master: name("br0"),
        },
    );

    // ASSERT
    assert_eq!(supervisor.status().owner, Some(name("br0")));
    assert!(seen.contains(&Seen::Configure {
        name: name("br0"),
        v4: true,
        v6: false,
    }));
}

#[test]
fn failed_ensure_bridge_keeps_the_primary_as_owner() {
    // ARRANGE
    let links = [link("eth0", 2, LinkState::Up)];
    let mut supervisor = Supervisor::new(config(vec![bridge()]));
    let now = Instant::now();
    run(&mut supervisor, now, Input::Command(Command::Initialize));
    run(&mut supervisor, now, Input::Links(&links));
    let provisioned = run(&mut supervisor, now, Input::Carriers(&links));
    assert!(provisioned.contains(&Seen::Bridge {
        name: name("br0"),
        stp: false,
        ports: vec!["auto".to_owned()],
    }));

    // ACT
    run(
        &mut supervisor,
        now,
        Input::EffectFailed {
            kind: EffectKind::EnsureBridge,
            error: io::Error::other("boom"),
        },
    );
    let reconciled = run(&mut supervisor, now, Input::Command(Command::Reconcile));

    // ASSERT
    assert_eq!(supervisor.status().owner, Some(name("eth0")));
    assert!(!provisioned.iter().any(
        |seen| matches!(seen, Seen::Configure { name: target, .. } if target == &name("br0"))
    ));
    assert!(!reconciled.iter().any(
        |seen| matches!(seen, Seen::Configure { name: target, .. } if target == &name("br0"))
    ));
}

#[test]
fn deleted_unknown_link_does_not_shutdown() {
    // ARRANGE
    let links = [link("eth0", 2, LinkState::Up)];
    let mut supervisor = boot(vec![ethernet("auto", true)], &links, &links);
    let now = Instant::now();

    // ACT
    let seen = run(
        &mut supervisor,
        now,
        Input::LinkEvent(netmodel::LinkEvent::Deleted {
            name: name("ghost0"),
            index: 9,
        }),
    );

    // ASSERT
    assert!(
        !seen
            .iter()
            .any(|effect| matches!(effect, Seen::Shutdown(_)))
    );
    assert_eq!(supervisor.status().primary, Some(name("eth0")));
}

#[test]
fn shutdown_clears_pending_actions() {
    // ARRANGE
    let links = [link("eth0", 2, LinkState::Up)];
    let mut supervisor = boot(vec![ethernet("auto", true)], &links, &links);
    let now = Instant::now();
    supervisor
        .queue
        .push_back(crate::actions::Action::EnsureBridge {
            index: 0,
            name: name("br0"),
        });

    // ACT
    let seen = run(&mut supervisor, now, Input::Command(Command::Shutdown));

    // ASSERT
    assert_eq!(seen.first(), Some(&Seen::Shutdown(name("eth0"))));
    assert!(
        !seen
            .iter()
            .any(|effect| matches!(effect, Seen::Bridge { .. }))
    );
    assert_eq!(supervisor.status().state, State::Uninitialized);
}

#[test]
fn failed_discovery_retries_with_backoff_and_success_clears_it() {
    // ARRANGE
    let mut supervisor = Supervisor::new(config(Vec::new()));
    let now = Instant::now();
    run(&mut supervisor, now, Input::Command(Command::Initialize));
    run(
        &mut supervisor,
        now,
        Input::EffectFailed {
            kind: EffectKind::Discover,
            error: io::Error::other("boom"),
        },
    );
    let first = supervisor.next_wake(now);

    // ACT
    let early = run(&mut supervisor, now, Input::Timer);
    let due = run(&mut supervisor, now + Duration::from_secs(1), Input::Timer);

    // ASSERT
    assert_eq!(first, now.checked_add(Duration::from_secs(1)));
    assert_eq!(early, Vec::new());
    assert_eq!(due.first(), Some(&Seen::Discover));

    // ACT
    run(
        &mut supervisor,
        now + Duration::from_secs(1),
        Input::EffectFailed {
            kind: EffectKind::Discover,
            error: io::Error::other("boom"),
        },
    );
    let backoff = supervisor.next_wake(now + Duration::from_secs(1));
    let links = [link("eth0", 2, LinkState::Up)];
    run(
        &mut supervisor,
        now + Duration::from_secs(1),
        Input::Links(&links),
    );

    // ASSERT
    assert_eq!(
        backoff,
        (now + Duration::from_secs(1)).checked_add(Duration::from_secs(2))
    );
    assert_eq!(
        supervisor.next_wake(now + Duration::from_secs(1)),
        now.checked_add(Duration::from_secs(30))
    );
}
