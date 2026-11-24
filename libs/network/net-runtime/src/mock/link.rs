//! Link port implementation for the deterministic mock runtime.

use std::io::Result;

use super::{Mock, State};

impl crate::Link for Mock {
    fn detach(&self, port: &netmodel::Name) -> impl Future<Output = Result<()>> + Send {
        std::future::ready(self.write(|state| detach(state, port)))
    }

    fn ensure_bridge(
        &self,
        name: &netmodel::Name,
        stp: bool,
    ) -> impl Future<Output = Result<()>> + Send {
        std::future::ready(self.write(|state| {
            ensure_bridge(state, name, stp);
            Ok(())
        }))
    }

    fn link(
        &self,
        name: &netmodel::Name,
    ) -> impl Future<Output = Result<Option<netmodel::Link>>> + Send {
        std::future::ready(Ok(
            self.read(|state| state.links.iter().find(|link| &link.name == name).cloned())
        ))
    }

    fn links(&self) -> impl Future<Output = Result<Vec<netmodel::Link>>> + Send {
        std::future::ready(Ok(self.read(|state| state.links.clone())))
    }

    fn set_down(&self, name: &netmodel::Name) -> impl Future<Output = Result<()>> + Send {
        std::future::ready(self.write(|state| set_state(state, name, netmodel::LinkState::Down)))
    }

    fn set_master(
        &self,
        port: &netmodel::Name,
        master: &netmodel::Name,
    ) -> impl Future<Output = Result<()>> + Send {
        std::future::ready(self.write(|state| set_master(state, port, master)))
    }

    fn set_up(&self, name: &netmodel::Name) -> impl Future<Output = Result<()>> + Send {
        std::future::ready(self.write(|state| set_state(state, name, netmodel::LinkState::Up)))
    }
}

fn detach(state: &mut State, port: &netmodel::Name) -> Result<()> {
    let link = state
        .links
        .iter_mut()
        .find(|link| &link.name == port)
        .ok_or_else(super::not_found)?;
    link.master = None;

    Ok(())
}

fn ensure_bridge(state: &mut State, name: &netmodel::Name, stp: bool) {
    record_bridge_stp(state, name, stp);
    if state.links.iter().any(|link| &link.name == name) {
        return;
    }

    let index = state.next_link_index;
    state.next_link_index = state.next_link_index.wrapping_add(1);
    state.links.push(netmodel::Link {
        name: name.clone(),
        index,
        mac: netmodel::Mac::default(),
        state: netmodel::LinkState::Down,
        is_virtual: true,
        master: None,
    });
}

fn record_bridge_stp(state: &mut State, name: &netmodel::Name, stp: bool) {
    match state.bridges.iter_mut().find(|entry| &entry.0 == name) {
        Some(entry) => entry.1 = stp,
        None => state.bridges.push((name.clone(), stp)),
    }
}

fn set_state(
    state: &mut State,
    name: &netmodel::Name,
    new_state: netmodel::LinkState,
) -> Result<()> {
    let link = state
        .links
        .iter_mut()
        .find(|link| &link.name == name)
        .ok_or_else(super::not_found)?;
    link.state = new_state;

    Ok(())
}

fn set_master(state: &mut State, port: &netmodel::Name, master: &netmodel::Name) -> Result<()> {
    let index = state
        .links
        .iter()
        .find(|link| &link.name == master)
        .map(|link| link.index)
        .ok_or_else(super::not_found)?;
    let link = state
        .links
        .iter_mut()
        .find(|link| &link.name == port)
        .ok_or_else(super::not_found)?;
    link.master = Some(index);

    Ok(())
}
