//! Link events applied to the controllers before the supervisor sees them.

use std::time::{Instant, SystemTime};

use net_runtime::Link as _;

use crate::error::Result;
use crate::pending::Pending;
use crate::{Loop, driver, feedback};

pub(crate) async fn link_event<R: net_runtime::Runtime>(
    driver: &mut Loop<R>,
    now: Instant,
    wall: SystemTime,
    event: netmodel::LinkEvent,
) -> Result<()> {
    let applied = if let Some(name) = event_name(&event).cloned() {
        apply_carrier(driver, now, wall, &event, &name).await
    } else {
        if let netmodel::LinkEvent::Deleted { ref name, .. } = event {
            driver::detach(driver, name);
        }
        Ok(())
    };
    driver
        .pending
        .push_back(Pending::SupervisorLinkEvent(event));

    applied
}

pub(crate) async fn interface_link<R: net_runtime::Runtime>(
    driver: &mut Loop<R>,
    now: Instant,
    wall: SystemTime,
    name: &netmodel::Name,
    info: ip_method::LinkInfo,
) -> Result<()> {
    if !driver.names.contains(name) {
        return Ok(());
    }

    feedback::interface_input(driver, now, wall, name, iface::Input::Link(info)).await
}

async fn apply_carrier<R: net_runtime::Runtime>(
    driver: &mut Loop<R>,
    now: Instant,
    wall: SystemTime,
    event: &netmodel::LinkEvent,
    name: &netmodel::Name,
) -> Result<()> {
    let Some(info) = event_link(driver, event).await? else {
        return Ok(());
    };
    let state = info.state;
    interface_link(driver, now, wall, name, info).await?;
    if let Some(master) = driver::owner_of(driver, name)
        && let Some(info) = driver::owner_link(driver, &master, state).await?
    {
        driver
            .pending
            .push_back(Pending::InterfaceLink { name: master, info });
    }

    Ok(())
}

async fn event_link<R: net_runtime::Runtime>(
    driver: &Loop<R>,
    event: &netmodel::LinkEvent,
) -> Result<Option<ip_method::LinkInfo>> {
    let Some(name) = event_name(event) else {
        return Ok(None);
    };
    let link = driver.runtime.link().link(name).await?;

    Ok(link.as_ref().map(driver::link_info))
}

const fn event_name(event: &netmodel::LinkEvent) -> Option<&netmodel::Name> {
    match *event {
        netmodel::LinkEvent::Up { ref name, .. }
        | netmodel::LinkEvent::Down { ref name, .. }
        | netmodel::LinkEvent::Added { ref name, .. } => Some(name),
        netmodel::LinkEvent::Deleted { .. } | netmodel::LinkEvent::Error(_) => None,
    }
}
