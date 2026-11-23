//! Global network supervisor finite state machine.

#![warn(missing_docs)]

use core::net::{IpAddr, Ipv4Addr, Ipv6Addr};
use std::time::Instant;

/// Readiness of the whole network subsystem.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum State {
    /// Supervisor has not started yet.
    #[default]
    Uninitialized,
    /// Supervisor is discovering links and provisioning interfaces.
    Initializing,
    /// At least one interface is provisioned and usable.
    Operational,
    /// Every configured interface is provisioned and stable.
    Ready,
    /// Provisioning failed or was degraded.
    Degraded,
}

/// Network configuration covering DNS, IPv6 toggle, and interfaces.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Config {
    /// Whether IPv6 networking is enabled.
    pub ipv6: bool,
    /// DNS server addresses for name resolution.
    pub dns: Vec<IpAddr>,
    /// Declarative list of network interfaces.
    pub interfaces: Vec<InterfaceConfig>,
}

/// Declarative configuration for a single network interface.
#[derive(Debug, Clone, PartialEq)]
pub struct InterfaceConfig {
    /// Interface name.
    pub name: String,
    /// Interface kind.
    pub kind: InterfaceKind,
    /// Optional IPv4 configuration.
    pub ipv4: Option<netmodel::Ipv4Config>,
    /// Optional IPv6 configuration.
    pub ipv6: Option<netmodel::Ipv6Config>,
    /// Optional bridge-specific configuration.
    pub bridge: Option<BridgeConfig>,
}

/// Kind of network interface.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InterfaceKind {
    /// Bridge interface connecting multiple networks.
    Bridge,
    /// Standard Ethernet interface.
    Ethernet,
}

/// Bridge-specific configuration.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct BridgeConfig {
    /// Member ports attached to the bridge.
    pub port: Vec<String>,
    /// Whether Spanning Tree Protocol is enabled.
    pub stp: bool,
}

/// Point-in-time status of the network supervisor.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Status {
    /// Global readiness.
    pub state: State,
    /// Elected uplink, i.e. the interface `auto` resolves to.
    pub primary: Option<netmodel::Name>,
    /// Interface that currently owns layer 3.
    pub owner: Option<netmodel::Name>,
    /// Link state reported by the interface that owns layer 3.
    pub owner_link: Option<netmodel::LinkState>,
    /// Uplink candidates in failover preference order.
    pub backups: Vec<netmodel::Name>,
    /// Netlink events dispatched since the supervisor started.
    pub processed_events: u64,
}

/// Commands the runtime sends to the supervisor.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Command {
    /// Discover links and provision the configured interfaces.
    Initialize,
    /// Reconcile desired and actual configuration.
    Reconcile,
    /// Stop every interface and shut the supervisor down.
    Shutdown,
}

/// Input the supervisor consumes on each step.
#[derive(Debug)]
pub enum Input<'a> {
    /// Poll the supervisor without new external input.
    Poll,
    /// Runtime command.
    Command(Command),
    /// Full link inventory.
    Links(&'a [netmodel::Link]),
    /// Carrier state per link index.
    Carriers(&'a [(u32, bool)]),
    /// Link lifecycle event.
    LinkEvent(netmodel::LinkEvent),
    /// Snapshot published by an interface controller.
    InterfaceSnapshot {
        /// Interface the snapshot belongs to.
        name: netmodel::Name,
        /// Snapshot published by the controller.
        snapshot: &'a iface::Snapshot,
    },
    /// Bridge reported ready.
    BridgeReady {
        /// Bridge name.
        name: netmodel::Name,
        /// Kernel interface index.
        index: u32,
        /// Hardware address.
        mac: netmodel::Mac,
        /// Current link state.
        state: netmodel::LinkState,
    },
    /// Port was enslaved to a bridge.
    Enslaved {
        /// Port that joined the bridge.
        port: netmodel::Name,
        /// Bridge the port joined.
        master: netmodel::Name,
    },
    /// Port was detached from its bridge.
    Detached {
        /// Port that left the bridge.
        port: netmodel::Name,
    },
    /// Runtime reported that an effect failed.
    EffectFailed {
        /// Effect that failed.
        kind: EffectKind,
        /// Error the runtime reported.
        error: std::io::Error,
    },
    /// Timer deadline elapsed.
    Timer,
}

/// Effect the supervisor requests from the runtime.
#[derive(Debug)]
pub enum Effect<'a> {
    /// Discover the current links.
    Discover,
    /// Probe carrier state for the given links.
    ProbeCarriers {
        /// Links to probe.
        links: &'a [netmodel::Link],
    },
    /// Configure one interface.
    ConfigureInterface {
        /// Interface to configure.
        name: netmodel::Name,
        /// IPv4 configuration to apply.
        v4: Option<&'a netmodel::Ipv4Config>,
        /// IPv6 configuration to apply.
        v6: Option<&'a netmodel::Ipv6Config>,
    },
    /// Ensure the bridge exists with the given configuration.
    EnsureBridge {
        /// Bridge to create or update.
        name: netmodel::Name,
        /// Bridge configuration to apply.
        config: &'a BridgeConfig,
    },
    /// Enslave a port to a bridge.
    Enslave {
        /// Port to enslave.
        port: netmodel::Name,
        /// Bridge the port joins.
        master: netmodel::Name,
    },
    /// Detach a port from its bridge.
    Detach {
        /// Port to detach.
        port: netmodel::Name,
    },
    /// Shut one interface down.
    ShutdownInterface {
        /// Interface to shut down.
        name: netmodel::Name,
    },
    /// Publish the supervisor status.
    Publish,
    /// Write the resolver configuration.
    WriteResolver {
        /// IPv4 nameservers.
        v4: &'a [Ipv4Addr],
        /// IPv6 nameservers.
        v6: &'a [Ipv6Addr],
    },
}

/// Kind of effect the runtime executes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EffectKind {
    /// Link discovery.
    Discover,
    /// Carrier probing.
    ProbeCarriers,
    /// Interface configuration.
    ConfigureInterface,
    /// Bridge creation or update.
    EnsureBridge,
    /// Port enslavement.
    Enslave,
    /// Port detachment.
    Detach,
    /// Interface shutdown.
    ShutdownInterface,
    /// Status publication.
    Publish,
    /// Resolver configuration write.
    WriteResolver,
}

/// Outcome of one supervisor step.
#[derive(Debug)]
pub enum Step<'a> {
    /// The supervisor produced an effect for the runtime.
    Effect(Effect<'a>),
    /// The supervisor is waiting for input or for its wake deadline.
    Wait,
}

/// Supervisor driving the global network state machine.
#[derive(Debug)]
pub struct Supervisor {
    config: Config,
    status: Status,
}

impl Supervisor {
    /// Creates a supervisor for the given configuration with an idle status.
    #[must_use = "the supervisor owns the network state"]
    pub fn new(config: Config) -> Self {
        Self {
            config,
            status: Status::default(),
        }
    }

    /// Performs at most one step and returns the next effect.
    pub fn step(&mut self, _now: Instant, _input: Input<'_>) -> Step<'_> {
        Step::Wait
    }

    /// Earliest instant at which the supervisor must be stepped again.
    #[must_use = "the caller needs the deadline to schedule the wake"]
    pub const fn next_wake(&self, _now: Instant) -> Option<Instant> {
        None
    }

    /// Returns the supervisor's current status.
    #[must_use = "callers need the status to report network readiness"]
    pub fn status(&self) -> Status {
        self.status.clone()
    }

    /// Returns the configuration the supervisor runs on.
    #[must_use = "callers need the configuration to reconcile interfaces"]
    pub const fn config(&self) -> &Config {
        &self.config
    }
}

#[cfg(test)]
mod tests {
    use core::net::{IpAddr, Ipv4Addr, Ipv6Addr};
    use std::io;
    use std::time::Instant;

    use netmodel::{LinkState, Mac, Name};

    use crate::{
        BridgeConfig, Command, Config, Effect, EffectKind, Input, InterfaceConfig, InterfaceKind,
        State, Status, Step, Supervisor,
    };

    #[test]
    fn default_config_and_status() {
        // ARRANGE
        let config = Config::default();
        let status = Status::default();

        // ACT
        let initial = Supervisor::new(Config::default()).status();

        // ASSERT
        assert!(!config.ipv6);
        assert_eq!(config.dns, Vec::<IpAddr>::new());
        assert_eq!(config.interfaces, Vec::<InterfaceConfig>::new());
        assert_eq!(status.state, State::Uninitialized);
        assert!(status.primary.is_none());
        assert!(status.owner.is_none());
        assert!(status.owner_link.is_none());
        assert_eq!(status.backups, Vec::<Name>::new());
        assert_eq!(status.processed_events, 0);
        assert_eq!(status, initial);
    }

    #[test]
    fn new_starts_uninitialized_and_step_waits() {
        // ARRANGE
        let config = Config {
            ipv6: true,
            dns: vec![IpAddr::V4(Ipv4Addr::new(9, 9, 9, 9))],
            interfaces: vec![InterfaceConfig {
                name: "br0".to_owned(),
                kind: InterfaceKind::Bridge,
                ipv4: None,
                ipv6: None,
                bridge: Some(BridgeConfig {
                    port: vec!["eth0".to_owned()],
                    stp: false,
                }),
            }],
        };
        let mut supervisor = Supervisor::new(config);
        let now = Instant::now();

        // ACT
        let status = supervisor.status();
        let waits = matches!(supervisor.step(now, Input::Poll), Step::Wait);

        // ASSERT
        assert_eq!(status.state, State::Uninitialized);
        assert!(waits);
        assert!(supervisor.next_wake(now).is_none());
        assert_eq!(supervisor.status().state, State::Uninitialized);
    }

    #[test]
    fn config_returns_the_stored_config() {
        // ARRANGE
        let config = Config {
            ipv6: true,
            dns: vec![IpAddr::V4(Ipv4Addr::new(9, 9, 9, 9))],
            interfaces: Vec::new(),
        };
        let supervisor = Supervisor::new(config);

        // ACT
        let stored = supervisor.config();

        // ASSERT
        assert!(stored.ipv6);
        assert_eq!(stored.dns, vec![IpAddr::V4(Ipv4Addr::new(9, 9, 9, 9))]);
        assert_eq!(stored.interfaces, Vec::<InterfaceConfig>::new());
    }

    #[test]
    fn inputs_and_effects_can_be_constructed() {
        // ARRANGE
        let name = Name::new("eth0").expect("valid name");
        let master = Name::new("br0").expect("valid name");
        let links = vec![netmodel::Link {
            name: name.clone(),
            index: 2,
            mac: Mac::default(),
            state: LinkState::Up,
            is_virtual: false,
            master: None,
        }];
        let config = BridgeConfig::default();
        let snapshot = iface::Snapshot {
            link: LinkState::Up,
            v4: None,
            v4_state: ip_method::State::Stopped,
            v6: None,
            v6_state: ip_method::State::Stopped,
        };
        let v4_dns = [Ipv4Addr::new(9, 9, 9, 9)];
        let v6_dns = [Ipv6Addr::LOCALHOST];

        // ACT
        let inputs = [
            Input::Command(Command::Initialize),
            Input::Links(&links),
            Input::Carriers(&[(2, true)]),
            Input::BridgeReady {
                name: master.clone(),
                index: 3,
                mac: Mac::default(),
                state: LinkState::Up,
            },
            Input::InterfaceSnapshot {
                name: name.clone(),
                snapshot: &snapshot,
            },
            Input::Enslaved {
                port: name.clone(),
                master: master.clone(),
            },
            Input::Detached { port: name.clone() },
            Input::EffectFailed {
                kind: EffectKind::EnsureBridge,
                error: io::Error::other("boom"),
            },
            Input::Timer,
        ];
        let effects = [
            Effect::Discover,
            Effect::ProbeCarriers { links: &links },
            Effect::ConfigureInterface {
                name: name.clone(),
                v4: None,
                v6: None,
            },
            Effect::EnsureBridge {
                name: master.clone(),
                config: &config,
            },
            Effect::Enslave {
                port: name.clone(),
                master,
            },
            Effect::Detach { port: name.clone() },
            Effect::ShutdownInterface { name },
            Effect::WriteResolver {
                v4: &v4_dns,
                v6: &v6_dns,
            },
            Effect::Publish,
        ];

        // ASSERT
        assert_eq!(inputs.len(), 9);
        assert_eq!(effects.len(), 9);
        assert!(matches!(inputs.first(), Some(Input::Command(_))));
        assert!(matches!(effects.first(), Some(Effect::Discover)));
    }
}
