//! Pending inputs fed back into the supervisor and the interface controllers.

use std::io;
use std::time::{Instant, SystemTime};

use net_runtime::Link as _;

use crate::pending::Pending;
use crate::{Loop, driver, events, step};

// Feeds one pending input to the FSM that consumes it.
pub(crate) async fn feed<R: net_runtime::Runtime>(
    driver: &mut Loop<R>,
    pending: Pending,
    now: Instant,
    wall: SystemTime,
) -> io::Result<()> {
    match pending {
        Pending::Links(links) => discover(driver, now, wall, &links).await?,
        Pending::Carriers(carriers) => {
            step::drive(driver, now, net_supervisor::Input::Carriers(&carriers)).await;
        }
        Pending::Snapshot { name, snapshot } => {
            let input = net_supervisor::Input::InterfaceSnapshot {
                name,
                snapshot: &snapshot,
            };
            step::drive(driver, now, input).await;
        }
        Pending::BridgeReady {
            name,
            index,
            mac,
            state,
        } => {
            driver::ensure(
                driver,
                ip_method::LinkInfo {
                    name: name.clone(),
                    index,
                    mac,
                    state,
                },
            );
            let input = net_supervisor::Input::BridgeReady {
                name,
                index,
                mac,
                state,
            };
            step::drive(driver, now, input).await;
        }
        Pending::Enslaved { port, master } => {
            enslave(driver, now, &port, &master).await?;
        }
        Pending::Detached { port } => {
            driver::detach(driver, &port);
            step::drive(driver, now, net_supervisor::Input::Detached { port }).await;
        }
        Pending::ConfigureInterface { name, v4, v6 } => {
            command(
                driver,
                now,
                wall,
                &name,
                iface::Command::Configure { v4, v6 },
            )
            .await?;
        }
        Pending::ReconcileInterface { name } => {
            command(driver, now, wall, &name, iface::Command::Reconcile).await?;
        }
        Pending::ShutdownInterface { name } => shutdown(driver, now, wall, &name).await?,
        Pending::InterfaceLink { name, info } => {
            events::interface_link(driver, now, wall, &name, info).await?;
        }
        Pending::LinkEvent(event) => events::link_event(driver, now, wall, event).await?,
        Pending::SupervisorLinkEvent(event) => {
            step::drive(driver, now, net_supervisor::Input::LinkEvent(event)).await;
        }
        Pending::EffectFailed { kind, error } => {
            let input = net_supervisor::Input::EffectFailed { kind, error };
            step::drive(driver, now, input).await;
        }
        Pending::Entropy {
            name,
            family,
            value,
        } => {
            let input = iface::Input::Method {
                family,
                input: ip_method::Input::Entropy(value),
            };
            interface_input(driver, now, wall, &name, input).await?;
        }
    }

    Ok(())
}

// Adds a controller for every link discovery reported and refreshes the ones
// that already existed so a re-added link gets its new index and state.
async fn discover<R: net_runtime::Runtime>(
    driver: &mut Loop<R>,
    now: Instant,
    wall: SystemTime,
    links: &[netmodel::Link],
) -> io::Result<()> {
    let mut first_error = None;
    for link in links {
        let info = driver::link_info(link);
        if driver::ensure(driver, info.clone()) {
            continue;
        }
        let name = info.name.clone();
        first_error = first_error.or(events::interface_link(driver, now, wall, &name, info)
            .await
            .err());
    }
    step::drive(driver, now, net_supervisor::Input::Links(links)).await;
    match first_error {
        Some(error) => Err(error),
        None => Ok(()),
    }
}

// Records the bridge that owns a port and makes the bridge reflect its carrier.
async fn enslave<R: net_runtime::Runtime>(
    driver: &mut Loop<R>,
    now: Instant,
    port: &netmodel::Name,
    master: &netmodel::Name,
) -> io::Result<()> {
    driver::enslave(driver, port.clone(), master.clone());
    if let Some(link) = driver.runtime.link().link(port).await?
        && let Some(info) = driver::owner_link(driver, master, link.state).await?
    {
        driver.pending.push_back(Pending::InterfaceLink {
            name: master.clone(),
            info,
        });
    }
    step::drive(
        driver,
        now,
        net_supervisor::Input::Enslaved {
            port: port.clone(),
            master: master.clone(),
        },
    )
    .await;

    Ok(())
}

// Shuts an interface down, tolerating a link the driver never saw.
async fn shutdown<R: net_runtime::Runtime>(
    driver: &mut Loop<R>,
    now: Instant,
    wall: SystemTime,
    name: &netmodel::Name,
) -> io::Result<()> {
    if !driver.names.contains(name) {
        return Ok(());
    }

    command(driver, now, wall, name, iface::Command::Shutdown).await
}

async fn command<R: net_runtime::Runtime>(
    driver: &mut Loop<R>,
    now: Instant,
    wall: SystemTime,
    name: &netmodel::Name,
    command: iface::Command,
) -> io::Result<()> {
    interface_input(driver, now, wall, name, iface::Input::Command(command)).await
}

// Feeds one input to a known controller and rejects unknown links.
pub(crate) async fn interface_input<R: net_runtime::Runtime>(
    driver: &mut Loop<R>,
    now: Instant,
    wall: SystemTime,
    name: &netmodel::Name,
    input: iface::Input<'_>,
) -> io::Result<()> {
    let Some(index) = driver.names.iter().position(|known| known == name) else {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            "no interface controller for the input",
        ));
    };
    step::advance(driver, index, input, now, wall).await?;

    Ok(())
}
