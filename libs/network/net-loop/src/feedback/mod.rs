//! Pending inputs fed back into the supervisor and the interface controllers.

mod socket;

use std::io;
use std::time::{Instant, SystemTime};

use net_runtime::Link as _;

use crate::pending::Pending;
use crate::{Loop, driver, events, step};

pub(crate) async fn feed<R: net_runtime::Runtime>(
    driver: &mut Loop<R>,
    pending: Pending,
    now: Instant,
    wall: SystemTime,
) -> io::Result<()> {
    match pending {
        Pending::Links(links) => discover(driver, now, wall, &links).await,
        Pending::Carriers(links) => {
            step::drive(driver, now, net_supervisor::Input::Carriers(&links)).await;
            Ok(())
        }
        Pending::Snapshot { name, snapshot } => {
            let input = net_supervisor::Input::InterfaceSnapshot {
                name,
                snapshot: &snapshot,
            };
            step::drive(driver, now, input).await;
            Ok(())
        }
        Pending::BridgeReady {
            name,
            index,
            mac,
            state,
        } => {
            let info = ip_method::LinkInfo {
                name: name.clone(),
                index,
                mac,
                state,
            };
            driver::ensure(driver, info);
            let input = net_supervisor::Input::BridgeReady {
                name,
                index,
                mac,
                state,
            };
            step::drive(driver, now, input).await;
            Ok(())
        }
        Pending::Enslaved { port, master } => enslave(driver, now, &port, &master).await,
        Pending::Detached { port } => {
            driver::detach(driver, &port);
            step::drive(driver, now, net_supervisor::Input::Detached { port }).await;
            Ok(())
        }
        Pending::ConfigureInterface { name, v4, v6 } => {
            let configure = iface::Command::Configure { v4, v6 };
            command(driver, now, wall, &name, configure).await
        }
        Pending::ReconcileInterface { name } => {
            command(driver, now, wall, &name, iface::Command::Reconcile).await
        }
        Pending::ShutdownInterface { name } => shutdown(driver, now, wall, &name).await,
        Pending::InterfaceLink { name, info } => {
            events::interface_link(driver, now, wall, &name, info).await
        }
        Pending::LinkEvent(event) => events::link_event(driver, now, wall, event).await,
        Pending::SupervisorLinkEvent(event) => {
            step::drive(driver, now, net_supervisor::Input::LinkEvent(event)).await;
            Ok(())
        }
        Pending::EffectFailed { kind, error } => {
            let input = net_supervisor::Input::EffectFailed { kind, error };
            step::drive(driver, now, input).await;
            Ok(())
        }
        other @ (Pending::Entropy { .. }
        | Pending::SocketOpened { .. }
        | Pending::SendCompleted { .. }
        | Pending::SocketClosed { .. }
        | Pending::SocketError { .. }
        | Pending::Frame { .. }) => socket::feed(driver, other, now, wall).await,
    }
}

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

async fn enslave<R: net_runtime::Runtime>(
    driver: &mut Loop<R>,
    now: Instant,
    port: &netmodel::Name,
    master: &netmodel::Name,
) -> io::Result<()> {
    driver::enslave(driver, port.clone(), master.clone());
    if let Some(link) = driver.runtime.link().link(port).await?
        && let Some(info) = driver::owner_link(driver, master, link.state()).await?
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
