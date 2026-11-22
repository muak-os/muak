//! Address provisioning state machine vocabulary.

pub mod fixed;
#[cfg(feature = "ipv4")]
pub mod v4;
#[cfg(feature = "ipv6")]
pub mod v6;

use alloc::string::String;
use core::future::Future;
use core::net::IpAddr;
use std::time::Instant;

use thiserror::Error;

/// Address configuration an interface method should apply.
#[derive(Debug, Clone)]
pub struct AddressConfig {
    /// Address to assign to the interface.
    pub address: IpAddr,
    /// Prefix length in bits.
    pub prefix_len: u8,
}

/// Outcome a method asks the controller to apply or remove.
#[derive(Debug, Clone)]
pub enum Intent {
    /// Apply the given address configuration.
    Configure(AddressConfig),
    /// Remove the given address configuration.
    Deconfigure(AddressConfig),
    /// Ensure a default route through the given gateway.
    EnsureDefaultRoute(IpAddr),
    /// Remove the default route through the given gateway.
    RemoveDefaultRoute(IpAddr),
}

/// Lifecycle phase of an address method.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum State {
    /// Acquiring address configuration.
    Acquiring,
    /// Configuration is bound and stable.
    Bound,
    /// Running with degraded configuration.
    Degraded,
    /// Method is stopped.
    Stopped,
}

/// Outcome of one address method step.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Step {
    /// The step changed state or queued events, so another step must follow.
    Advance,
    /// The step is waiting for I/O or for its wake deadline.
    Wait,
}

/// Commands the controller sends to an address method.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Command {
    /// Start the method.
    Start,
    /// Stop the method.
    Stop,
    /// Reconcile desired and actual configuration.
    Reconcile,
    /// Link transitioned to up.
    LinkUp,
    /// Link transitioned to down.
    LinkDown,
}

/// Failures reported by an address method.
#[derive(Debug, Error)]
pub enum Failure {
    /// Socket input or output failed.
    #[error("method I/O failed: {0}")]
    Io(#[source] std::io::Error),
    /// Protocol exchange failed.
    #[error("method protocol failed: {0}")]
    Protocol(String),
}

/// Event an address method emits to the controller.
#[derive(Debug)]
pub enum Event {
    /// Method produced an intent for the controller to apply.
    Intent(Intent),
    /// Method changed lifecycle phase.
    State(State),
    /// Method reported a non-fatal issue.
    Warning(String),
    /// Method failed.
    Failed(Failure),
}

/// Address method driving one family on one interface.
pub trait Method {
    /// Point-in-time state the method exposes to the controller.
    type View;

    /// Performs at most one I/O step and returns the next event.
    ///
    /// Never sleeps. `now` is the controller's clock reading for this step.
    /// Every deadline at or before `now` must be handled before awaiting I/O.
    fn next_event(&mut self, now: Instant) -> impl Future<Output = Event> + Send;

    /// Earliest instant at which the method must be polled again with no I/O pending.
    ///
    /// Returning an earlier instant is safe; a later one is a bug. Returning `now`
    /// asks the controller for an immediate step.
    fn next_wake(&self, now: Instant) -> Option<Instant>;

    /// Applies a command from the controller.
    fn on_command(&mut self, command: Command);

    /// Returns the method's current view, or `None` when it has nothing to report.
    fn view(&self) -> Option<Self::View>;
}

#[cfg(test)]
mod tests {
    use super::State;

    #[test]
    fn state_variants_are_distinct() {
        // ARRANGED
        let bound = State::Bound;
        let stopped = State::Stopped;

        // ACT
        let equal = bound == stopped;

        // ASSERT
        assert!(!equal);
    }
}
