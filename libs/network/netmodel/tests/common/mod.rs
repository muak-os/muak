//! Shared link fixtures for netmodel integration tests.

#![cfg(test)]

use netmodel::{AdminState, CarrierState, Link, LinkState, Mac, Name};

/// Builds a link with the given name and state.
#[must_use = "the test needs the link to select against"]
pub(crate) fn link(name: &str, state: LinkState) -> Link {
    link_with_index(name, 0, state)
}

/// Builds a link with the given name, index and state.
#[must_use = "the test needs the link to select against"]
pub(crate) fn link_with_index(name: &str, index: u32, state: LinkState) -> Link {
    link_with_parts(name, index, state.admin(), state.carrier())
}

/// Builds a link with explicit administrative and carrier parts.
#[must_use = "the test needs the link to select against"]
pub(crate) fn link_with_parts(
    name: &str,
    index: u32,
    admin: AdminState,
    carrier: CarrierState,
) -> Link {
    Link {
        name: Name::new(name).expect("valid name"),
        index,
        mac: Mac::default(),
        admin,
        carrier,
        is_virtual: false,
        master: None,
    }
}
