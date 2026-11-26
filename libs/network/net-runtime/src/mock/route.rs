//! Default route port implementation for the deterministic mock runtime.

use std::io::Result;

use super::{Mock, State};
use crate::Route;

impl Route for Mock {
    fn ensure_default(
        &self,
        link: &netmodel::Name,
        gateway: core::net::IpAddr,
    ) -> impl Future<Output = Result<()>> + Send {
        std::future::ready(self.write(|state| ensure(state, link, gateway)))
    }

    fn remove_default(
        &self,
        link: &netmodel::Name,
        gateway: core::net::IpAddr,
    ) -> impl Future<Output = Result<()>> + Send {
        std::future::ready(self.write(|state| remove(state, link, gateway)))
    }
}

fn ensure(state: &mut State, link: &netmodel::Name, gateway: core::net::IpAddr) -> Result<()> {
    super::require_link(state, link)?;
    let entry = (link.clone(), gateway);
    if !state.default_routes.contains(&entry) {
        state.default_routes.push(entry);
    }

    Ok(())
}

fn remove(state: &mut State, link: &netmodel::Name, gateway: core::net::IpAddr) -> Result<()> {
    super::require_link(state, link)?;
    state
        .default_routes
        .retain(|entry| !(&entry.0 == link && entry.1 == gateway));

    Ok(())
}
