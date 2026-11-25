//! Sans-io observation journal for network state machines.
//!
//! The journal records structured observations and leaves rendering to the
//! caller, so the state machines stay free of logging, sockets, and runtimes.

#![warn(missing_docs)]

extern crate alloc;

use alloc::collections::VecDeque;
use core::fmt;

use netmodel::{AdminState, CarrierState, Link, LinkEvent, LinkState, Name};

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
    /// Returns the level the effect is reported at.
    ///
    /// High frequency effects report at `Debug`, everything else at `Info`.
    #[must_use = "observability needs the level to report the effect"]
    pub const fn level(self) -> Level {
        match self {
            Self::ProbeCarriers | Self::Publish | Self::WriteResolver => Level::Debug,
            Self::Discover
            | Self::SetLinkUp
            | Self::ConfigureInterface
            | Self::ReconcileInterface
            | Self::EnsureBridge
            | Self::Enslave
            | Self::Detach
            | Self::ShutdownInterface => Level::Info,
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
            name: link.name.clone(),
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
}

/// One journaled event and the level it should be reported at.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Observation {
    /// Level the event should be reported at.
    pub level: Level,
    /// Event that was observed.
    pub event: Event,
}

/// FIFO journal of observations waiting to be rendered.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Journal {
    entries: VecDeque<Observation>,
}

impl Journal {
    /// Creates an empty journal.
    #[must_use = "the journal owns the observations"]
    pub const fn new() -> Self {
        Self {
            entries: VecDeque::new(),
        }
    }

    /// Appends one observation.
    pub fn record(&mut self, level: Level, event: Event) {
        self.entries.push_back(Observation { level, event });
    }

    /// Iterates over the observations in recording order and removes them.
    ///
    /// Observations recorded since the last drain accumulate until consumed.
    #[must_use = "unconsumed observations accumulate in the journal"]
    pub fn drain(&mut self) -> impl Iterator<Item = Observation> + '_ {
        self.entries.drain(..)
    }

    /// Returns true when no observation is recorded.
    #[must_use = "callers need to know whether there is anything to render"]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Returns the number of recorded observations.
    #[must_use = "callers need the backlog size"]
    pub fn len(&self) -> usize {
        self.entries.len()
    }
}

#[cfg(test)]
mod tests {
    use crate::{EffectKind, Event, Journal, Level, LinkSnapshot, SupervisorState};

    fn name(value: &str) -> netmodel::Name {
        netmodel::Name::new(value).expect("valid name")
    }

    #[test]
    fn new_journal_is_empty() {
        // ARRANGE
        let journal = Journal::new();

        // ACT
        let empty = journal.is_empty();
        let len = journal.len();

        // ASSERT
        assert!(empty);
        assert_eq!(len, 0);
    }

    #[test]
    fn record_keeps_order_and_levels() {
        // ARRANGE
        let mut journal = Journal::new();
        let state = Event::StateChanged {
            from: SupervisorState::Uninitialized,
            to: SupervisorState::Initializing,
        };
        let effect = Event::EffectStarted {
            kind: EffectKind::Discover,
            target: None,
        };

        // ACT
        journal.record(Level::Info, state.clone());
        journal.record(Level::Debug, effect.clone());
        let recorded = journal.drain().collect::<Vec<_>>();

        // ASSERT
        assert_eq!(journal.len(), 0);
        assert_eq!(recorded.len(), 2);
        assert_eq!(recorded[0].level, Level::Info);
        assert_eq!(recorded[0].event, state);
        assert_eq!(recorded[1].level, Level::Debug);
        assert_eq!(recorded[1].event, effect);
    }

    #[test]
    fn drain_empties_the_journal() {
        // ARRANGE
        let mut journal = Journal::new();
        journal.record(Level::Info, Event::Detached { port: name("eth0") });

        // ACT
        let first = journal.drain().count();
        let second = journal.drain().count();

        // ASSERT
        assert_eq!(first, 1);
        assert_eq!(second, 0);
        assert!(journal.is_empty());
    }

    #[test]
    fn snapshot_conversion_keeps_name_index_and_state() {
        // ARRANGE
        let link = netmodel::Link {
            name: name("eth0"),
            index: 2,
            mac: netmodel::Mac::default(),
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
    fn effect_levels_are_debug_only_for_high_frequency_effects() {
        // ARRANGE
        let cases = [
            (EffectKind::Discover, Level::Info),
            (EffectKind::ProbeCarriers, Level::Debug),
            (EffectKind::SetLinkUp, Level::Info),
            (EffectKind::ConfigureInterface, Level::Info),
            (EffectKind::ReconcileInterface, Level::Info),
            (EffectKind::EnsureBridge, Level::Info),
            (EffectKind::Enslave, Level::Info),
            (EffectKind::Detach, Level::Info),
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
