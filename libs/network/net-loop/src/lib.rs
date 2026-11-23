//! Runtime-agnostic driver loop for network finite state machines.
//!
//! The loop owns the global [`net_supervisor::Supervisor`], one
//! [`iface::Controller`] per known interface, and the logical socket table.
//! Each [`Loop::step`] feeds the FSMs one input at a time and executes the
//! effect they answer with against the [`net_runtime::Runtime`] ports, so the
//! same orchestration runs on any backend that implements the ports.

#![warn(missing_docs)]

extern crate alloc;

mod driver;
mod interface_effects;
mod pending;
mod supervisor_effects;

#[cfg(test)]
mod tests;

use alloc::collections::VecDeque;

use pending::Pending;

/// Interface and method slot a socket belongs to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SocketOwner {
    /// Interface the socket is bound to.
    pub interface: netmodel::Name,
    /// Address family that opened the socket.
    pub family: iface::Family,
    /// Method slot the socket occupies on the interface.
    pub slot: ip_method::SocketSlot,
    /// Kind of socket the runtime opened.
    pub kind: ip_method::SocketKind,
}

/// One runtime socket handle together with its owner.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SocketEntry {
    /// Handle the runtime returned when the socket was opened.
    pub id: net_runtime::SocketId,
    /// Interface and slot that own the handle.
    pub owner: SocketOwner,
}

/// Logical socket table mapping runtime handles to their owners.
#[derive(Debug, Default)]
pub struct SocketTable {
    entries: Vec<SocketEntry>,
}

impl SocketTable {
    /// Creates an empty socket table.
    #[must_use = "the table owns the socket bookkeeping"]
    pub const fn new() -> Self {
        Self {
            entries: Vec::new(),
        }
    }

    /// Inserts a socket, replacing and returning the owner of the same handle.
    pub fn insert(&mut self, id: net_runtime::SocketId, owner: SocketOwner) -> Option<SocketOwner> {
        if let Some(entry) = self.entries.iter_mut().find(|entry| entry.id == id) {
            return Some(core::mem::replace(&mut entry.owner, owner));
        }
        self.entries.push(SocketEntry { id, owner });

        None
    }

    /// Removes a socket and returns the owner it had.
    pub fn remove(&mut self, id: net_runtime::SocketId) -> Option<SocketOwner> {
        let index = self.entries.iter().position(|entry| entry.id == id)?;
        Some(self.entries.remove(index).owner)
    }

    /// Returns the owner of a socket handle.
    #[must_use = "callers need the owner to dispatch on the socket kind"]
    pub fn get(&self, id: net_runtime::SocketId) -> Option<&SocketOwner> {
        self.entries
            .iter()
            .find(|entry| entry.id == id)
            .map(|entry| &entry.owner)
    }

    /// Returns the handle owned by the given interface, family, and slot.
    #[must_use = "callers need the handle to send on the socket"]
    pub fn find(
        &self,
        interface: &netmodel::Name,
        family: iface::Family,
        slot: ip_method::SocketSlot,
    ) -> Option<net_runtime::SocketId> {
        self.entries
            .iter()
            .find(|entry| {
                &entry.owner.interface == interface
                    && entry.owner.family == family
                    && entry.owner.slot == slot
            })
            .map(|entry| entry.id)
    }

    /// Iterates over the sockets in insertion order.
    pub fn iter(&self) -> impl Iterator<Item = &SocketEntry> {
        self.entries.iter()
    }

    /// Returns the number of open sockets.
    #[must_use = "callers need the open socket count"]
    pub const fn len(&self) -> usize {
        self.entries.len()
    }

    /// Returns true when no socket is open.
    #[must_use = "callers need to know whether the table is empty"]
    pub const fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

/// Outcome of one driver step.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Progress {
    /// Neither the FSMs nor the socket table had work for this step.
    Idle,
    /// The step consumed an input or executed an effect.
    Worked,
}

/// Driver ownership of the runtime, the FSMs, and the socket table.
pub struct Loop<R: net_runtime::Runtime> {
    runtime: R,
    supervisor: net_supervisor::Supervisor,
    interfaces: Vec<iface::Controller>,
    // Interface names stay beside the controllers so a step can borrow the
    // name while the matching controller is stepped mutably. Both lists are
    // only ever appended to together in `add_interface`.
    names: Vec<netmodel::Name>,
    sockets: SocketTable,
    pending: VecDeque<Pending>,
}
