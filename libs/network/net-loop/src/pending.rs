//! Owned inputs produced by effect dispatch and fed back into the FSMs.

use alloc::vec::Vec;

/// Input the driver owns until the next step.
#[derive(Debug)]
pub(crate) enum Pending {
    /// Full link inventory returned by discovery.
    Links(Vec<netmodel::Link>),
    /// Full link inventory with fresh administrative and carrier state.
    Carriers(Vec<netmodel::Link>),
    /// Snapshot an interface controller published.
    Snapshot {
        /// Interface the snapshot belongs to.
        name: netmodel::Name,
        /// Snapshot the controller published.
        snapshot: iface::Snapshot,
    },
    /// Bridge the runtime created or updated.
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
    /// Port the runtime attached to a bridge.
    Enslaved {
        /// Port that joined the bridge.
        port: netmodel::Name,
        /// Bridge the port joined.
        master: netmodel::Name,
    },
    /// Port the runtime detached from its bridge.
    Detached {
        /// Port that left the bridge.
        port: netmodel::Name,
    },
    /// Per-family configuration the supervisor asked an interface to apply.
    ConfigureInterface {
        /// Interface to configure.
        name: netmodel::Name,
        /// IPv4 configuration to apply.
        v4: Option<netmodel::Ipv4Config>,
        /// IPv6 configuration to apply.
        v6: Option<netmodel::Ipv6Config>,
    },
    /// Reconcile request for one active interface.
    ReconcileInterface {
        /// Interface to reconcile.
        name: netmodel::Name,
    },
    /// Shutdown request for one interface.
    ShutdownInterface {
        /// Interface to shut down.
        name: netmodel::Name,
    },
    /// Link lifecycle event the runtime reported.
    LinkEvent(netmodel::LinkEvent),
    /// Link lifecycle event the supervisor consumes after the controllers.
    SupervisorLinkEvent(netmodel::LinkEvent),
    /// Link transition an interface controller has to apply.
    InterfaceLink {
        /// Interface to update.
        name: netmodel::Name,
        /// Link state the runtime reported.
        info: ip_method::LinkInfo,
    },
    /// Supervisor effect the runtime could not execute.
    EffectFailed {
        /// Effect that failed.
        kind: net_supervisor::EffectKind,
        /// Error the runtime reported.
        error: std::io::Error,
    },
    /// Entropy the runtime produced for one interface method.
    Entropy {
        /// Interface that requested the entropy.
        name: netmodel::Name,
        /// Address family that requested the entropy.
        family: iface::Family,
        /// Random value the runtime filled.
        value: u32,
    },
    /// Socket the runtime opened for one interface method.
    SocketOpened {
        /// Interface that owns the socket.
        name: netmodel::Name,
        /// Address family that owns the socket.
        family: iface::Family,
        /// Slot the socket occupies on the method.
        slot: ip_method::SocketSlot,
    },
    /// Send the runtime completed for one interface method.
    SendCompleted {
        /// Interface that owns the socket.
        name: netmodel::Name,
        /// Address family that owns the socket.
        family: iface::Family,
        /// Slot the send completed on.
        slot: ip_method::SocketSlot,
    },
    /// Socket the runtime closed for one interface method.
    SocketClosed {
        /// Interface that owned the socket.
        name: netmodel::Name,
        /// Address family that owned the socket.
        family: iface::Family,
        /// Slot the socket occupied on the method.
        slot: ip_method::SocketSlot,
    },
    /// Socket error the runtime reported for one interface method.
    SocketError {
        /// Interface that owns the socket.
        name: netmodel::Name,
        /// Address family that owns the socket.
        family: iface::Family,
        /// Slot that reported the error.
        slot: ip_method::SocketSlot,
        /// Error the runtime reported.
        error: std::io::Error,
    },
    /// Frame the runtime received on a socket of one interface method.
    Frame {
        /// Interface that owns the socket.
        name: netmodel::Name,
        /// Address family that owns the socket.
        family: iface::Family,
        /// Slot the frame arrived on.
        slot: ip_method::SocketSlot,
        /// Raw frame bytes.
        bytes: Vec<u8>,
        /// Source address the transport reported, when it reports one.
        source: Option<core::net::IpAddr>,
    },
}
