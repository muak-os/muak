//! Owned actions queued by the policy and rendered into borrowed effects.

use netmodel::Name;

/// One queued supervisor action.
///
/// Names are owned so the queue can outlive a step, while configuration stays
/// in the supervisor and is borrowed only when the action is rendered.
#[derive(Debug)]
pub(crate) enum Action {
    /// Discover the current links.
    Discover,
    /// Probe carrier state for the last discovered links.
    ProbeCarriers,
    /// Ensure the bridge at `index` exists.
    EnsureBridge {
        /// Index of the bridge interface configuration.
        index: usize,
        /// Bridge name.
        name: Name,
    },
    /// Enslave a port to a bridge.
    Enslave {
        /// Port to enslave.
        port: Name,
        /// Bridge the port joins.
        master: Name,
    },
    /// Detach a port from its bridge.
    Detach {
        /// Port to detach.
        port: Name,
    },
    /// Configure the interface at `index` through `target`.
    Configure {
        /// Index of the interface configuration.
        #[cfg(any(feature = "ipv4", feature = "ipv6"))]
        index: usize,
        /// Interface that owns layer 3.
        target: Name,
    },
    /// Ask an active interface to reconcile its methods.
    ReconcileInterface {
        /// Interface to reconcile.
        name: Name,
    },
    /// Shut an interface down.
    ShutdownInterface {
        /// Interface to shut down.
        name: Name,
    },
    /// Publish the supervisor status.
    Publish,
    /// Write the resolver configuration.
    WriteResolver,
}
