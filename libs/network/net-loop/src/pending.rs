//! Owned inputs produced by effect dispatch and fed back into the FSMs.

/// Input the driver owns until the next step.
///
/// Effects borrow the FSM that produced them, so anything the FSMs must see
/// again is copied into one of these variants first.
#[derive(Debug)]
pub(crate) enum Pending {
    /// Full link inventory returned by discovery.
    Links(Vec<netmodel::Link>),
    /// Carrier state per link index.
    Carriers(Vec<(u32, bool)>),
    /// Per-family configuration the supervisor asked an interface to apply.
    ConfigureInterface {
        /// Interface to configure.
        name: netmodel::Name,
        /// IPv4 configuration to apply.
        v4: Option<netmodel::Ipv4Config>,
        /// IPv6 configuration to apply.
        v6: Option<netmodel::Ipv6Config>,
    },
    /// Shutdown request for one interface.
    ShutdownInterface {
        /// Interface to shut down.
        name: netmodel::Name,
    },
}
