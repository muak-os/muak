//! Sans-io observation journal for network state machines.

#![warn(missing_docs)]

extern crate alloc;

pub mod interface;
pub mod journal;

use alloc::vec::Vec;
use core::fmt;
use core::net::IpAddr;

use netmodel::{AdminState, CarrierState, Family, Link, LinkEvent, LinkState, MethodState, Name};

/// Severity an observation should be reported at.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Level {
    /// Verbose diagnostic detail.
    Debug,
    /// Normal operational reporting.
    Info,
    /// Recoverable failure or unexpected condition.
    Warn,
    /// Unrecoverable failure.
    Error,
}

/// Readiness of the whole network subsystem.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SupervisorState {
    /// Supervisor has not started yet.
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

impl fmt::Display for SupervisorState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            Self::Uninitialized => f.write_str("Uninitialized"),
            Self::Initializing => f.write_str("Initializing"),
            Self::Operational => f.write_str("Operational"),
            Self::Ready => f.write_str("Ready"),
            Self::Degraded => f.write_str("Degraded"),
        }
    }
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

impl EffectKind {
    /// Returns Info for operator events and Debug for effect-trace noise.
    #[must_use = "observability needs the level to report the effect"]
    pub const fn level(self) -> Level {
        match self {
            Self::Discover
            | Self::SetLinkUp
            | Self::ConfigureInterface
            | Self::ShutdownInterface => Level::Info,
            Self::ProbeCarriers
            | Self::EnsureBridge
            | Self::Enslave
            | Self::Detach
            | Self::ReconcileInterface
            | Self::Publish
            | Self::WriteResolver => Level::Debug,
        }
    }
}

impl fmt::Display for EffectKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            Self::Discover => f.write_str("discover"),
            Self::ProbeCarriers => f.write_str("probe-carriers"),
            Self::SetLinkUp => f.write_str("set-link-up"),
            Self::ConfigureInterface => f.write_str("configure-interface"),
            Self::ReconcileInterface => f.write_str("reconcile-interface"),
            Self::EnsureBridge => f.write_str("ensure-bridge"),
            Self::Enslave => f.write_str("enslave"),
            Self::Detach => f.write_str("detach"),
            Self::ShutdownInterface => f.write_str("shutdown-interface"),
            Self::Publish => f.write_str("publish"),
            Self::WriteResolver => f.write_str("write-resolver"),
        }
    }
}

/// Administrative and carrier state of a discovered link.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LinkSnapshot {
    /// Interface name.
    pub name: Name,
    /// Kernel interface index.
    pub index: u32,
    /// Administrative state.
    pub admin: AdminState,
    /// Carrier state.
    pub carrier: CarrierState,
}

impl From<&Link> for LinkSnapshot {
    fn from(link: &Link) -> Self {
        Self {
            name: link.name,
            index: link.index,
            admin: link.admin,
            carrier: link.carrier,
        }
    }
}

/// A structured network event worth reporting to the operator.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Event {
    /// Supervisor readiness changed.
    StateChanged {
        /// Previous readiness.
        from: SupervisorState,
        /// New readiness.
        to: SupervisorState,
    },
    /// Elected primary or backup order changed.
    PrimaryChanged {
        /// Elected uplink, i.e. the interface `auto` resolves to.
        primary: Option<Name>,
        /// Uplink candidates in failover preference order.
        backups: Vec<Name>,
    },
    /// Layer 3 owner or owner link state changed.
    OwnerChanged {
        /// Interface that currently owns layer 3.
        owner: Option<Name>,
        /// Link state reported by the interface that owns layer 3.
        link: Option<LinkState>,
    },
    /// The initial physical link inventory was discovered.
    LinksDiscovered {
        /// Discovered links.
        links: Vec<LinkSnapshot>,
    },
    /// Link monitoring reported a lifecycle event.
    LinkEvent {
        /// Event reported by the monitor.
        event: LinkEvent,
    },
    /// A bridge reported itself ready.
    BridgeReady {
        /// Bridge name.
        name: Name,
        /// Kernel interface index.
        index: u32,
        /// Current link state.
        state: LinkState,
    },
    /// A port joined a bridge.
    Enslaved {
        /// Port that joined the bridge.
        port: Name,
        /// Bridge the port joined.
        master: Name,
    },
    /// A port left its bridge.
    Detached {
        /// Port that left the bridge.
        port: Name,
    },
    /// The runtime started executing an effect.
    EffectStarted {
        /// Kind of effect being executed.
        kind: EffectKind,
        /// Interface or port the effect targets, if any.
        target: Option<Name>,
    },
    /// The runtime finished an effect successfully.
    EffectFinished {
        /// Kind of effect that finished.
        kind: EffectKind,
        /// Interface or port the effect targeted, if any.
        target: Option<Name>,
    },
    /// The runtime failed to execute an effect.
    EffectFailed {
        /// Kind of effect that failed.
        kind: EffectKind,
        /// Interface or port the effect targeted, if any.
        target: Option<Name>,
        /// Error the runtime reported.
        error: String,
    },
    /// One address family of an interface changed provisioning state.
    InterfaceState {
        /// Interface the method runs on.
        name: Name,
        /// Address family of the method.
        family: Family,
        /// New provisioning state.
        state: MethodState,
    },
    /// An interface method reported a warning or failure.
    Notice {
        /// Interface that reported the notice.
        name: Name,
        /// Address family of the reporting method, when known.
        family: Option<Family>,
        /// Notice text.
        text: String,
    },
    /// An address was assigned to an interface.
    AddressAssigned {
        /// Interface the address was assigned to.
        name: Name,
        /// Assigned address.
        address: IpAddr,
        /// Prefix length in bits.
        prefix_len: u8,
    },
    /// An address was removed from an interface.
    AddressRemoved {
        /// Interface the address was removed from.
        name: Name,
        /// Removed address.
        address: IpAddr,
        /// Prefix length in bits.
        prefix_len: u8,
    },
    /// A default route was ensured on an interface.
    RouteEnsured {
        /// Interface the route goes through.
        name: Name,
        /// Gateway of the default route.
        gateway: IpAddr,
    },
    /// A default route was removed from an interface.
    RouteRemoved {
        /// Interface the route was removed from.
        name: Name,
        /// Gateway of the default route.
        gateway: IpAddr,
    },
    /// A `DHCPv4` method reported its lease.
    DhcpLease {
        /// Interface that owns the lease.
        name: Name,
        /// Lease details and lifecycle phase.
        lease: interface::DhcpLease,
    },
    /// A SLAAC method reported its learned configuration.
    SlaacUpdate {
        /// Interface that reported the update.
        name: Name,
        /// Addresses, routers, and DNS servers the method learned.
        update: interface::SlaacUpdate,
    },
}

/// One journaled event and the level it should be reported at.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Observation {
    /// Level the event should be reported at.
    pub level: Level,
    /// Event that was observed.
    pub event: Event,
}

#[cfg(test)]
mod tests {
    use crate::{EffectKind, Level, LinkSnapshot, SupervisorState};

    fn name(value: &str) -> netmodel::Name {
        netmodel::Name::new(value).expect("valid name")
    }

    #[test]
    fn snapshot_conversion_keeps_name_index_and_state() {
        // ARRANGE
        let link = netmodel::Link {
            name: name("eth0"),
            index: 2,
            mac: mac::Mac::default(),
            admin: netmodel::AdminState::Up,
            carrier: netmodel::CarrierState::Down,
            is_virtual: false,
            master: None,
        };

        // ACT
        let snapshot = LinkSnapshot::from(&link);

        // ASSERT
        assert_eq!(snapshot.name, name("eth0"));
        assert_eq!(snapshot.index, 2);
        assert_eq!(snapshot.admin, netmodel::AdminState::Up);
        assert_eq!(snapshot.carrier, netmodel::CarrierState::Down);
    }

    #[test]
    fn effect_levels_separate_operator_events_from_trace_noise() {
        // ARRANGE
        let cases = [
            (EffectKind::Discover, Level::Info),
            (EffectKind::ProbeCarriers, Level::Debug),
            (EffectKind::SetLinkUp, Level::Info),
            (EffectKind::ConfigureInterface, Level::Info),
            (EffectKind::ReconcileInterface, Level::Debug),
            (EffectKind::EnsureBridge, Level::Debug),
            (EffectKind::Enslave, Level::Debug),
            (EffectKind::Detach, Level::Debug),
            (EffectKind::ShutdownInterface, Level::Info),
            (EffectKind::Publish, Level::Debug),
            (EffectKind::WriteResolver, Level::Debug),
        ];

        // ACT
        let levels: Vec<(EffectKind, Level)> = cases
            .iter()
            .map(|&(kind, _)| (kind, kind.level()))
            .collect();

        // ASSERT
        assert_eq!(levels, cases.to_vec());
    }

    #[test]
    fn effect_display_is_kebab_case() {
        // ACT / ASSERT
        assert_eq!(EffectKind::ProbeCarriers.to_string(), "probe-carriers");
        assert_eq!(EffectKind::SetLinkUp.to_string(), "set-link-up");
        assert_eq!(
            EffectKind::ConfigureInterface.to_string(),
            "configure-interface"
        );
        assert_eq!(EffectKind::WriteResolver.to_string(), "write-resolver");
    }

    #[test]
    fn supervisor_state_display_matches_the_names() {
        // ACT / ASSERT
        assert_eq!(SupervisorState::Uninitialized.to_string(), "Uninitialized");
        assert_eq!(SupervisorState::Initializing.to_string(), "Initializing");
        assert_eq!(SupervisorState::Operational.to_string(), "Operational");
        assert_eq!(SupervisorState::Ready.to_string(), "Ready");
        assert_eq!(SupervisorState::Degraded.to_string(), "Degraded");
    }
}
