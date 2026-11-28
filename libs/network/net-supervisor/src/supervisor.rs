//! Supervisor construction, stepping, rendering, and status queries.

use alloc::collections::VecDeque;
use std::collections::{HashMap, HashSet};
use std::time::Instant;

use netmodel::BridgeConfig;
#[cfg(any(feature = "ipv4", feature = "ipv6"))]
use netmodel::{InterfaceConfig, InterfaceKind};

use crate::actions::Action;
use crate::state::Snapshot;
use crate::{Command, Effect, Input, State, Status, Step, Supervisor, dispatch, observe, owner};

/// Fields that trigger a status publication when any of them changes.
pub(crate) type StatusKey = (
    State,
    Option<netmodel::Name>,
    Option<netmodel::Name>,
    Option<netmodel::LinkState>,
    Vec<netmodel::Name>,
);

impl Supervisor {
    /// Creates a supervisor for the given configuration with an idle status.
    #[must_use = "the supervisor owns the network state"]
    pub fn new(config: netmodel::Config) -> Self {
        Self {
            config,
            state: Snapshot::default(),
            links: Vec::new(),
            known: HashSet::new(),
            physical: HashSet::new(),
            snapshots: HashMap::new(),
            owners: HashMap::new(),
            enslaved: HashMap::new(),
            queue: VecDeque::new(),
            resolver_v4: Vec::new(),
            resolver_v6: Vec::new(),
            empty_bridge: BridgeConfig::default(),
            #[cfg(any(feature = "ipv4", feature = "ipv6"))]
            empty_interface: InterfaceConfig {
                name: String::new(),
                kind: InterfaceKind::Ethernet,
                ipv4: None,
                ipv6: None,
                bridge: None,
            },
            processed_events: 0,
            reconcile_at: None,
            retry_at: None,
            retry_attempts: 0,
            published: (State::Uninitialized, None, None, None, Vec::new()),
        }
    }

    /// Performs at most one step, records what changed, and returns the next effect.
    pub fn step(
        &mut self,
        now: Instant,
        input: Input<'_>,
        journal: &mut net_observe::journal::Journal,
    ) -> Step<'_> {
        let before = self.status_key();
        let publish = !matches!(&input, Input::Command(Command::Shutdown));
        dispatch::handle(self, now, input, journal);
        let key = self.status_key();
        observe::record(journal, &before, &key);
        for (from, to) in core::mem::take(&mut self.state.transitions) {
            journal.record(
                net_observe::Level::Info,
                net_observe::Event::StateChanged {
                    from: from.into(),
                    to: to.into(),
                },
            );
        }

        if publish && key != self.published {
            self.published = key;
            self.queue.push_back(Action::Publish);
        }

        match self.queue.pop_front() {
            Some(action) => self.render(&action),
            None => Step::Wait,
        }
    }

    /// Earliest instant at which the supervisor must be stepped again.
    #[must_use = "the caller needs the deadline to schedule the wake"]
    pub fn next_wake(&self, now: Instant) -> Option<Instant> {
        if !self.queue.is_empty() {
            return Some(now);
        }

        match (self.reconcile_at, self.retry_at) {
            (Some(reconcile), Some(retry)) => Some(reconcile.min(retry)),
            (Some(reconcile), None) => Some(reconcile),
            (None, Some(retry)) => Some(retry),
            (None, None) => None,
        }
    }

    /// Returns the supervisor's current status.
    #[must_use = "callers need the status to report network readiness"]
    pub fn status(&self) -> Status {
        let owner = owner::owner(self).copied();
        let owner_link = owner
            .as_ref()
            .and_then(|owner| self.snapshots.get(owner))
            .map(|snapshot| snapshot.link);

        Status {
            state: self.state.state,
            primary: self.state.primary,
            owner,
            owner_link,
            backups: self.state.backups.clone(),
            processed_events: self.processed_events,
        }
    }

    /// Returns the configuration the supervisor runs on.
    #[must_use = "callers need the configuration to reconcile interfaces"]
    pub const fn config(&self) -> &netmodel::Config {
        &self.config
    }

    fn status_key(&self) -> StatusKey {
        let owner = owner::owner(self).copied();
        let owner_link = owner
            .as_ref()
            .and_then(|owner| self.snapshots.get(owner))
            .map(|snapshot| snapshot.link);

        (
            self.state.state,
            self.state.primary,
            owner,
            owner_link,
            self.state.backups.clone(),
        )
    }

    fn render(&self, action: &Action) -> Step<'_> {
        let effect = match *action {
            Action::Discover => Effect::Discover,
            Action::ProbeCarriers => Effect::ProbeCarriers,
            Action::SetLinkUp { name } => Effect::SetLinkUp { name },
            Action::EnsureBridge { index, name } => {
                let config = self
                    .config
                    .interfaces
                    .get(index)
                    .and_then(|config| config.bridge.as_ref())
                    .unwrap_or(&self.empty_bridge);
                Effect::EnsureBridge { name, config }
            }
            Action::Enslave { port, master } => Effect::Enslave { port, master },
            Action::Detach { port } => Effect::Detach { port },
            #[cfg(any(feature = "ipv4", feature = "ipv6"))]
            Action::Configure { index, target } => {
                let config = self
                    .config
                    .interfaces
                    .get(index)
                    .unwrap_or(&self.empty_interface);
                #[cfg(not(feature = "ipv4"))]
                let v4 = None;
                #[cfg(feature = "ipv4")]
                let v4 = config.ipv4.as_ref();
                #[cfg(not(feature = "ipv6"))]
                let v6 = None;
                #[cfg(feature = "ipv6")]
                let v6 = config.ipv6.as_ref();
                Effect::ConfigureInterface {
                    name: target,
                    v4,
                    v6,
                }
            }
            #[cfg(not(any(feature = "ipv4", feature = "ipv6")))]
            Action::Configure { target, .. } => Effect::ConfigureInterface {
                name: target,
                v4: None,
                v6: None,
            },
            Action::ReconcileInterface { name } => Effect::ReconcileInterface { name },
            Action::ShutdownInterface { name } => Effect::ShutdownInterface { name },
            Action::Publish => Effect::Publish,
            Action::WriteResolver => Effect::WriteResolver {
                v4: &self.resolver_v4,
                v6: &self.resolver_v6,
            },
        };

        Step::Effect(effect)
    }
}
