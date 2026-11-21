//! Address provisioning state machine vocabulary.

#[cfg(feature = "ipv4")]
pub mod v4;
#[cfg(feature = "ipv6")]
pub mod v6;

use alloc::string::String;
use core::future::Future;
use core::net::IpAddr;

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
    /// Method failed.
    Failed(Failure),
}

/// Address method driving one family on one interface.
pub trait Method {
    /// Point-in-time state the method exposes to the controller.
    type View;

    /// Returns the next event produced by the method.
    fn next_event(&mut self) -> impl Future<Output = Event> + Send;

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
