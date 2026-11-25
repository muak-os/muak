//! Controller stepping, method routing, and effect translation.

mod command;
mod drive;
mod event;
mod observe;

use alloc::collections::VecDeque;
use alloc::vec::Vec;
use std::time::{Instant, SystemTime};

use ip_method::State;

use crate::{Controller, Input, Snapshot, Step};

impl Controller {
    /// Creates a controller for the given link with an idle snapshot.
    #[must_use = "the controller owns the interface state"]
    pub const fn new(link: ip_method::LinkInfo) -> Self {
        let snapshot = Snapshot {
            link: link.state,
            v4: None,
            v4_state: State::Stopped,
            v6: None,
            v6_state: State::Stopped,
            notices: Vec::new(),
        };

        Self {
            link,
            snapshot,
            v4: None,
            v6: None,
            pending: VecDeque::new(),
            deferred: None,
            shutdown: false,
        }
    }

    /// Performs at most one step and returns the next effect.
    pub fn step(
        &mut self,
        journal: &mut net_observe::journal::Journal,
        now: Instant,
        wall: SystemTime,
        input: Input<'_>,
    ) -> Step<'_> {
        drive::dispatch(self, journal, now, wall, input)
    }

    /// Earliest instant at which the controller must be stepped again.
    #[must_use = "the caller needs the deadline to schedule the wake"]
    pub fn next_wake(&self, now: Instant) -> Option<Instant> {
        drive::wake(self, now)
    }

    /// Returns the controller's current snapshot.
    #[must_use = "callers need the snapshot to report interface state"]
    pub const fn snapshot(&self) -> &Snapshot {
        &self.snapshot
    }

    /// Returns the link the controller runs on.
    #[must_use = "callers need the link to address interface effects"]
    pub const fn link(&self) -> &ip_method::LinkInfo {
        &self.link
    }
}

impl core::fmt::Debug for Controller {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Controller")
            .field("link", &self.link)
            .field("snapshot", &self.snapshot)
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests;
