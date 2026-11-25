//! Driver construction, accessors, and explicit start.

use alloc::collections::VecDeque;
use core::net::{Ipv4Addr, Ipv6Addr};
use std::io;
use std::time::{Instant, SystemTime};

use net_runtime::Link as _;

use crate::resolver::Nameservers;
use crate::{Progress, step};

impl<R: net_runtime::Runtime> crate::Loop<R> {
    /// Creates a driver loop for the runtime and supervisor configuration.
    ///
    /// The loop does not start the supervisor: callers run [`crate::Loop::start`]
    /// before stepping so callers that only step keep working.
    #[must_use = "the loop owns the FSMs and must be stepped"]
    pub fn new(runtime: R, config: net_supervisor::Config) -> Self {
        Self {
            runtime,
            supervisor: net_supervisor::Supervisor::new(config),
            journal: net_observe::journal::Journal::new(),
            interfaces: Vec::new(),
            names: Vec::new(),
            owners: Vec::new(),
            sockets: crate::SocketTable::new(),
            resolver: Nameservers::default(),
            started: false,
            pending: VecDeque::new(),
        }
    }

    /// Feeds the supervisor its initialization command and executes the effect.
    ///
    /// Services call this once before entering the step loop. Calling it again
    /// is a no-op, so a reconnecting service cannot restart discovery.
    pub async fn start(&mut self, now: Instant) -> Progress {
        step::start(self, now).await
    }

    /// Adds an interface controller for the given link when it is unknown.
    pub fn add_interface(&mut self, link: ip_method::LinkInfo) {
        ensure(self, link);
    }

    /// Returns the interface controllers in insertion order.
    #[must_use = "callers need the interfaces to inspect their state"]
    pub fn interfaces(&self) -> &[iface::Controller] {
        &self.interfaces
    }

    /// Returns the logical socket table.
    #[must_use = "callers need the table to inspect the open sockets"]
    pub const fn sockets(&self) -> &crate::SocketTable {
        &self.sockets
    }

    /// Returns the runtime the loop drives.
    #[must_use = "callers need the runtime to drive its ports"]
    pub const fn runtime(&self) -> &R {
        &self.runtime
    }

    /// Returns the supervisor status the last step reached.
    #[must_use = "callers need the status to report network readiness"]
    pub fn status(&self) -> net_supervisor::Status {
        self.supervisor.status()
    }

    /// Returns the nameservers of the last resolver write the supervisor asked for.
    #[must_use = "callers need the nameservers to write the resolver configuration"]
    pub fn resolver(&self) -> (&[Ipv4Addr], &[Ipv6Addr]) {
        (self.resolver.v4(), self.resolver.v6())
    }

    /// Drains the observations the loop recorded since the last drain.
    #[must_use = "unconsumed observations accumulate in the journal"]
    pub fn drain_observations(&mut self) -> impl Iterator<Item = net_observe::Observation> + '_ {
        self.journal.drain()
    }

    /// Earliest instant at which the driver must step again.
    #[must_use = "the caller needs the deadline to schedule the wake"]
    pub fn next_wake(&self, now: Instant) -> Option<Instant> {
        if !self.started {
            return None;
        }

        if !self.pending.is_empty() {
            return Some(now);
        }

        let mut next = self.supervisor.next_wake(now);
        for controller in &self.interfaces {
            next = match (next, controller.next_wake(now)) {
                (Some(supervisor), Some(controller)) => Some(supervisor.min(controller)),
                (deadline, None) | (None, deadline) => deadline,
            };
        }

        next
    }

    /// Feeds at most one input to the FSMs and executes the resulting effect.
    ///
    /// # Errors
    ///
    /// Returns the error a runtime port reported while executing an interface
    /// effect or feeding a pending input back. Supervisor effect failures are
    /// fed back to the supervisor instead of being returned.
    pub async fn step(&mut self, now: Instant, wall: SystemTime) -> io::Result<Progress> {
        step::run(self, now, wall).await
    }

    /// Waits for the next socket frame or link event and queues its handling.
    ///
    /// # Errors
    ///
    /// Returns the error the link event port reported.
    pub async fn wait_event(&mut self) -> io::Result<Progress> {
        crate::pump::wait_event(self).await
    }
}

/// Ensures exactly one controller exists per interface name.
pub(crate) fn ensure<R: net_runtime::Runtime>(
    driver: &mut crate::Loop<R>,
    link: ip_method::LinkInfo,
) -> bool {
    if driver.names.contains(&link.name) {
        return false;
    }

    driver.names.push(link.name.clone());
    driver.interfaces.push(iface::Controller::new(link));

    true
}

/// Records that a port was enslaved to a bridge.
pub(crate) fn enslave<R: net_runtime::Runtime>(
    driver: &mut crate::Loop<R>,
    port: netmodel::Name,
    master: netmodel::Name,
) {
    if let Some(entry) = driver.owners.iter_mut().find(|entry| entry.0 == port) {
        entry.1 = master;
        return;
    }

    driver.owners.push((port, master));
}

/// Forgets every port that is, or belongs to, the deleted link.
pub(crate) fn detach<R: net_runtime::Runtime>(driver: &mut crate::Loop<R>, name: &netmodel::Name) {
    driver
        .owners
        .retain(|entry| &entry.0 != name && &entry.1 != name);
}

/// Returns the bridge that owns the given port.
pub(crate) fn owner_of<R: net_runtime::Runtime>(
    driver: &crate::Loop<R>,
    port: &netmodel::Name,
) -> Option<netmodel::Name> {
    driver
        .owners
        .iter()
        .find(|entry| &entry.0 == port)
        .map(|entry| entry.1.clone())
}

/// Builds the owner link view carrying the carrier state of one of its ports.
pub(crate) async fn owner_link<R: net_runtime::Runtime>(
    driver: &crate::Loop<R>,
    owner: &netmodel::Name,
    state: netmodel::LinkState,
) -> io::Result<Option<ip_method::LinkInfo>> {
    let Some(link) = driver.runtime.link().link(owner).await? else {
        return Ok(None);
    };
    let info = ip_method::LinkInfo {
        name: owner.clone(),
        index: link.index,
        mac: link.mac,
        state,
    };

    Ok(Some(info))
}

/// Converts a discovered link into the controller view of that link.
pub(crate) fn link_info(link: &netmodel::Link) -> ip_method::LinkInfo {
    ip_method::LinkInfo {
        name: link.name.clone(),
        index: link.index,
        mac: link.mac,
        state: link.state(),
    }
}
