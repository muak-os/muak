//! Global network supervisor finite state machine.

#![warn(missing_docs)]

extern crate alloc;

mod actions;
mod discovery;
mod dispatch;
mod dns;
mod failover;
mod feedback;
mod links;
mod observe;
mod owner;
mod provision;
mod reconcile;
mod retry;
mod state;
mod supervisor;
mod view;

use alloc::collections::VecDeque;
use core::net::{IpAddr, Ipv4Addr, Ipv6Addr};
use std::collections::{HashMap, HashSet};

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
    /// Full link inventory with fresh administrative and carrier state.
    Carriers(&'a [netmodel::Link]),
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
    /// Probe carrier state after re-reading the link state.
    ProbeCarriers,
    /// Bring one administratively down link up.
    SetLinkUp {
        /// Link to activate.
        name: netmodel::Name,
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
    /// Ask an active interface to reconcile its methods.
    ReconcileInterface {
        /// Interface to reconcile.
        name: netmodel::Name,
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
    /// Link activation.
    SetLinkUp,
    /// Interface configuration.
    ConfigureInterface,
    /// Interface reconciliation.
    ReconcileInterface,
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

impl Effect<'_> {
    /// Returns the interface or port this effect targets, if any.
    #[must_use = "observability needs the target to report the effect"]
    pub const fn target(&self) -> Option<&netmodel::Name> {
        match *self {
            Self::SetLinkUp { ref name }
            | Self::ConfigureInterface { ref name, .. }
            | Self::ReconcileInterface { ref name }
            | Self::EnsureBridge { ref name, .. }
            | Self::ShutdownInterface { ref name } => Some(name),
            Self::Enslave { ref port, .. } | Self::Detach { ref port } => Some(port),
            Self::Discover | Self::ProbeCarriers | Self::Publish | Self::WriteResolver { .. } => {
                None
            }
        }
    }

    /// Returns the kind of work this effect asks the runtime to perform.
    #[must_use = "the driver needs the kind to report an effect failure"]
    pub const fn kind(&self) -> EffectKind {
        match *self {
            Self::Discover => EffectKind::Discover,
            Self::ProbeCarriers => EffectKind::ProbeCarriers,
            Self::SetLinkUp { .. } => EffectKind::SetLinkUp,
            Self::ConfigureInterface { .. } => EffectKind::ConfigureInterface,
            Self::ReconcileInterface { .. } => EffectKind::ReconcileInterface,
            Self::EnsureBridge { .. } => EffectKind::EnsureBridge,
            Self::Enslave { .. } => EffectKind::Enslave,
            Self::Detach { .. } => EffectKind::Detach,
            Self::ShutdownInterface { .. } => EffectKind::ShutdownInterface,
            Self::Publish => EffectKind::Publish,
            Self::WriteResolver { .. } => EffectKind::WriteResolver,
        }
    }
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
    pub(crate) config: Config,
    pub(crate) state: state::Snapshot,
    pub(crate) links: Vec<netmodel::Link>,
    pub(crate) known: HashSet<netmodel::Name>,
    pub(crate) physical: HashSet<netmodel::Name>,
    pub(crate) snapshots: HashMap<netmodel::Name, iface::Snapshot>,
    pub(crate) owners: HashMap<netmodel::Name, netmodel::Name>,
    pub(crate) enslaved: HashMap<netmodel::Name, netmodel::Name>,
    pub(crate) queue: VecDeque<actions::Action>,
    pub(crate) resolver_v4: Vec<Ipv4Addr>,
    pub(crate) resolver_v6: Vec<Ipv6Addr>,
    pub(crate) empty_bridge: BridgeConfig,
    #[cfg(any(feature = "ipv4", feature = "ipv6"))]
    pub(crate) empty_interface: InterfaceConfig,
    pub(crate) processed_events: u64,
    pub(crate) reconcile_at: Option<std::time::Instant>,
    pub(crate) retry_at: Option<std::time::Instant>,
    pub(crate) retry_attempts: u32,
    pub(crate) published: supervisor::StatusKey,
}
