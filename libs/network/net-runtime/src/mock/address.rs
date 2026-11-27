//! Address port implementation for the deterministic mock runtime.

use super::{Mock, State};
use crate::Address;
use crate::error::Result;

impl Address for Mock {
    fn add(
        &self,
        link: &netmodel::Name,
        address: netmodel::Address,
    ) -> impl Future<Output = Result<()>> + Send {
        std::future::ready(self.write(|state| add(state, link, address)))
    }

    fn list(
        &self,
        link: &netmodel::Name,
    ) -> impl Future<Output = Result<Vec<netmodel::Address>>> + Send {
        std::future::ready(Ok(self.read(|state| list(state, link))))
    }

    fn remove(
        &self,
        link: &netmodel::Name,
        address: netmodel::Address,
    ) -> impl Future<Output = Result<()>> + Send {
        std::future::ready(self.write(|state| remove(state, link, address)))
    }
}

fn add(state: &mut State, link: &netmodel::Name, address: netmodel::Address) -> Result<()> {
    super::require_link(state, link)?;
    let entry = (link.clone(), address);
    if !state.addresses.contains(&entry) {
        state.addresses.push(entry);
    }

    Ok(())
}

fn list(state: &State, link: &netmodel::Name) -> Vec<netmodel::Address> {
    state
        .addresses
        .iter()
        .filter(|entry| &entry.0 == link)
        .map(|entry| entry.1)
        .collect()
}

fn remove(state: &mut State, link: &netmodel::Name, address: netmodel::Address) -> Result<()> {
    super::require_link(state, link)?;
    state
        .addresses
        .retain(|entry| !(&entry.0 == link && entry.1 == address));

    Ok(())
}
