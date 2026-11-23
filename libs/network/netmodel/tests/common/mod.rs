//! Shared link fixtures for netmodel integration tests.

#![cfg(test)]

use netmodel::{Link, LinkState, Mac, Name};

/// Builds a link with the given name and state.
#[must_use = "the test needs the link to select against"]
pub(crate) fn link(name: &str, state: LinkState) -> Link {
    link_with_index(name, 0, state)
}

/// Builds a link with the given name, index and state.
#[must_use = "the test needs the link to select against"]
pub(crate) fn link_with_index(name: &str, index: u32, state: LinkState) -> Link {
    Link {
        name: Name::new(name).expect("valid name"),
        index,
        mac: Mac::default(),
        state,
        is_virtual: false,
        master: None,
    }
}
