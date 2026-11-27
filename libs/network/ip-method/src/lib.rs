//! Family-neutral IP provisioning method contract.

#![warn(missing_docs)]

pub mod error;

use core::net::{IpAddr, SocketAddr};
use std::io;
use std::time::{Instant, SystemTime};

use netmodel::{LinkState, Mac, MethodState, Name};

use crate::error::IpMethodError;

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

/// Address configuration an interface method should apply.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AddressConfig {
    /// Address to assign to the interface.
    pub address: IpAddr,
    /// Prefix length in bits.
    pub prefix_len: u8,
}

/// Outcome a method asks the controller to apply or remove.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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

/// Event an address method emits to the controller.
#[derive(Debug)]
pub enum Event {
    /// Method produced an intent for the controller to apply.
    Intent(Intent),
    /// Method changed lifecycle phase.
    State(MethodState),
    /// Method reported a nonfatal issue.
    Warning(String),
    /// Method failed.
    Failed(IpMethodError),
}

/// Slot an address method uses for one logical socket.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SocketSlot {
    /// Primary socket used for the main protocol exchange.
    Primary,
    /// Secondary socket used alongside the primary one.
    Secondary,
}

/// Kind of socket the runtime must open.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SocketKind {
    /// Raw layer 2 socket.
    RawL2,
    /// UDP socket.
    Udp,
    /// Raw `ICMPv6` socket.
    RawIcmpv6,
}

/// Specification for a socket the runtime must open.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SocketSpec {
    /// Slot the method uses to refer to the socket.
    pub slot: SocketSlot,
    /// Kind of socket to open.
    pub kind: SocketKind,
    /// Link the socket is bound to.
    pub link: Name,
    /// Optional local address to bind.
    pub bind: Option<SocketAddr>,
    /// Optional peer address to connect.
    pub peer: Option<SocketAddr>,
}

/// A snapshot of the link an address method runs on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LinkInfo {
    /// Interface name.
    pub name: Name,
    /// Kernel interface index.
    pub index: u32,
    /// Hardware address.
    pub mac: Mac,
    /// Current link state.
    pub state: LinkState,
}

/// Input an address method consumes on each step.
#[derive(Debug)]
pub enum Input<'a> {
    /// Poll the method without new external input.
    Poll,
    /// Controller command.
    Command(Command),
    /// Link snapshot update.
    Link(LinkInfo),
    /// Frame received on a socket.
    Frame {
        /// Slot the frame was received on.
        slot: SocketSlot,
        /// Raw frame bytes.
        bytes: &'a [u8],
        /// Source address of the received frame, when the transport reports one. Raw L2 and UDP sockets may report
        /// `None`; a raw `ICMPv6` socket reports the IPv6 source address.
        source: Option<IpAddr>,
    },
    /// Socket finished opening.
    SocketOpened {
        /// Slot of the opened socket.
        slot: SocketSlot,
    },
    /// Socket closed.
    SocketClosed {
        /// Slot of the closed socket.
        slot: SocketSlot,
    },
    /// Pending send completed.
    SendCompleted {
        /// Slot the send completed on.
        slot: SocketSlot,
    },
    /// Socket reported an error.
    SocketError {
        /// Slot that reported the error.
        slot: SocketSlot,
        /// Error reported by the socket.
        error: io::Error,
    },
    /// Entropy was provided by the runtime.
    ///
    /// Each request supplies at least 32 bits. A method needing more must request again or derive additional bits from
    /// other inputs.
    Entropy(u32),
    /// Timer deadline elapsed.
    Timer,
}

/// Effect an address method requests from the runtime.
#[derive(Debug)]
pub enum Effect<'a> {
    /// Open a socket described by the specification.
    OpenSocket(SocketSpec),
    /// Send a frame on an already open socket.
    ///
    /// `dst` is `None` for a [`SocketKind::RawL2`] socket, whose Ethernet destination is inside the frame, and for a
    /// [`SocketKind::Udp`] socket, which is connected through [`SocketSpec::peer`]. A [`SocketKind::RawIcmpv6`] socket
    /// requires `Some(SocketAddr::V6(..))` because the frame does not carry its destination.
    Send {
        /// Slot to send the frame on.
        slot: SocketSlot,
        /// Raw frame bytes to send.
        frame: &'a [u8],
        /// Destination address, required only for `ICMPv6` sockets.
        dst: Option<SocketAddr>,
    },
    /// Close an open socket.
    CloseSocket {
        /// Slot to close.
        slot: SocketSlot,
    },
    /// Request fresh entropy from the runtime.
    ///
    /// Each request supplies at least 32 bits. A method needing more must request again or derive additional bits from
    /// other inputs.
    RequestEntropy,
    /// Emit an event to the controller.
    Emit(Event),
}

/// Outcome of one address method step.
#[derive(Debug)]
pub enum Step<'a> {
    /// The method produced an effect for the runtime.
    Effect(Effect<'a>),
    /// The method is waiting for input or for its wake deadline.
    Wait,
}

/// Address method driving one family on one interface.
pub trait Method {
    /// Point-in-time state the method exposes to the controller.
    type View;

    /// Earliest instant at which the method must be polled again with no I/O pending. Returning an earlier instant is
    /// safe; a later one is a bug. Returning `now` asks the controller for an immediate step.
    fn next_wake(&self, now: Instant) -> Option<Instant>;

    /// Performs at most one step and returns the next effect. Every deadline at or before `now` must be handled before
    /// waiting for I/O.
    fn step(&mut self, now: Instant, wall: SystemTime, input: Input<'_>) -> Step<'_>;

    /// Returns the method's current view, or `None` when it has nothing to report.
    fn view(&self) -> Option<Self::View>;
}

#[cfg(test)]
mod tests {
    use netmodel::MethodState;

    #[test]
    fn state_variants_are_distinct() {
        // ARRANGE
        let bound = MethodState::Bound;
        let stopped = MethodState::Stopped;

        // ACT
        let equal = bound == stopped;

        // ASSERT
        assert!(!equal);
    }
}
