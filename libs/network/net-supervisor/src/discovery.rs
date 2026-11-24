//! Link inventory discovery, carrier probing, and provisioning kickoff.

use std::collections::HashSet;
use std::time::Instant;

use netmodel::{Link, Name, Selector};

use crate::actions::Action;
use crate::{State, Supervisor, dns, provision, retry};

/// Applies a full link inventory and re-elects the primary.
pub(crate) fn on_links(supervisor: &mut Supervisor, now: Instant, links: &[Link]) {
    if supervisor.state.state == State::Uninitialized {
        return;
    }

    let physical: Vec<Link> = links
        .iter()
        .filter(|link| !link.is_virtual && link.is_ethernet())
        .cloned()
        .collect();

    if supervisor.state.is_operational() {
        refresh(supervisor, physical);
        retry::succeeded(supervisor);
        dns::flush(supervisor);
        return;
    }

    supervisor.links.clone_from(&physical);
    supervisor.physical = physical.iter().map(|link| link.name.clone()).collect();
    for link in &physical {
        supervisor.known.insert(link.name.clone());
    }

    if supervisor.state.state == State::Degraded {
        supervisor.state.transition(State::Initializing);
    }
    supervisor.state.primary = None;
    supervisor.state.l3_owner = None;
    supervisor.state.backups.clear();

    if physical.is_empty() {
        supervisor.state.ensure_degraded();
        retry::schedule(supervisor, now);
        dns::flush(supervisor);
        return;
    }

    retry::succeeded(supervisor);
    supervisor.queue.push_back(Action::ProbeCarriers);
}

/// Applies carrier state and provisions the elected interfaces.
pub(crate) fn on_carriers(supervisor: &mut Supervisor, now: Instant, carriers: &[(u32, bool)]) {
    if supervisor.state.is_operational() || supervisor.state.state == State::Uninitialized {
        return;
    }

    for link in &mut supervisor.links {
        link.state = carrier_state(carrier_for(carriers, link.index));
    }

    if !supervisor.links.iter().any(Link::has_carrier) {
        supervisor.state.ensure_degraded();
        retry::schedule(supervisor, now);
        dns::flush(supervisor);
        return;
    }

    let Some(primary) = Selector::primary(&supervisor.links).map(|link| link.name.clone()) else {
        supervisor.state.ensure_degraded();
        retry::schedule(supervisor, now);
        return;
    };
    let backups: Vec<Name> = Selector::backups(&supervisor.links, &primary)
        .iter()
        .map(|link| link.name.clone())
        .collect();
    supervisor.state.primary = Some(primary);
    supervisor.state.backups = backups;

    supervisor.state.ensure_operational();
    retry::clear(supervisor);
    dns::flush(supervisor);
    provision::provision_all(supervisor);
    supervisor.state.transition(State::Ready);
}

fn refresh(supervisor: &mut Supervisor, physical: Vec<Link>) {
    let present: HashSet<Name> = physical.iter().map(|link| link.name.clone()).collect();
    let vanished: Vec<Name> = supervisor
        .physical
        .iter()
        .filter(|name| !present.contains(*name))
        .cloned()
        .collect();
    for name in vanished {
        supervisor.physical.remove(&name);
        supervisor.known.remove(&name);
        supervisor.links.retain(|link| link.name != name);
        supervisor.state.backups.retain(|backup| backup != &name);
    }

    for link in physical {
        let name = link.name.clone();
        if let Some(existing) = supervisor.links.iter_mut().find(|entry| entry.name == name) {
            *existing = link;
            continue;
        }
        supervisor.links.push(link);
        if supervisor.physical.contains(&name) {
            continue;
        }
        supervisor.physical.insert(name.clone());
        supervisor.known.insert(name.clone());
        if supervisor.state.primary.as_ref() != Some(&name)
            && !supervisor.state.backups.contains(&name)
        {
            supervisor.state.backups.push(name);
        }
    }
}

fn carrier_for(carriers: &[(u32, bool)], index: u32) -> bool {
    carriers
        .iter()
        .copied()
        .find_map(|(carrier_index, carrier)| (carrier_index == index).then_some(carrier))
        .unwrap_or(false)
}

const fn carrier_state(carrier: bool) -> netmodel::LinkState {
    if carrier {
        netmodel::LinkState::Up
    } else {
        netmodel::LinkState::NoCarrier
    }
}
