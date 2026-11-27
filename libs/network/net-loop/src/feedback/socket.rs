//! Socket pending events fed back into the address method that owns the socket.

use std::io;
use std::time::{Instant, SystemTime};

use crate::pending::Pending;
use crate::{Loop, feedback};

pub(super) async fn feed<R: net_runtime::Runtime>(
    driver: &mut Loop<R>,
    pending: Pending,
    now: Instant,
    wall: SystemTime,
) -> io::Result<()> {
    match pending {
        Pending::Entropy {
            name,
            family,
            value,
        } => {
            let input = ip_method::Input::Entropy(value);
            method(driver, now, wall, &name, family, input).await
        }
        Pending::SocketOpened { name, family, slot } => {
            let input = ip_method::Input::SocketOpened { slot };
            method(driver, now, wall, &name, family, input).await
        }
        Pending::SendCompleted { name, family, slot } => {
            let input = ip_method::Input::SendCompleted { slot };
            method(driver, now, wall, &name, family, input).await
        }
        Pending::SocketClosed { name, family, slot } => {
            let input = ip_method::Input::SocketClosed { slot };
            method(driver, now, wall, &name, family, input).await
        }
        Pending::SocketError {
            name,
            family,
            slot,
            error,
        } => {
            let input = ip_method::Input::SocketError { slot, error };
            method(driver, now, wall, &name, family, input).await
        }
        Pending::Frame {
            name,
            family,
            slot,
            bytes,
            source,
        } => {
            let input = ip_method::Input::Frame {
                slot,
                bytes: &bytes,
                source,
            };
            method(driver, now, wall, &name, family, input).await
        }
        Pending::Links(_)
        | Pending::Carriers(_)
        | Pending::Snapshot { .. }
        | Pending::BridgeReady { .. }
        | Pending::Enslaved { .. }
        | Pending::Detached { .. }
        | Pending::ConfigureInterface { .. }
        | Pending::ReconcileInterface { .. }
        | Pending::ShutdownInterface { .. }
        | Pending::InterfaceLink { .. }
        | Pending::LinkEvent(_)
        | Pending::SupervisorLinkEvent(_)
        | Pending::EffectFailed { .. } => Ok(()),
    }
}

async fn method<R: net_runtime::Runtime>(
    driver: &mut Loop<R>,
    now: Instant,
    wall: SystemTime,
    name: &netmodel::Name,
    family: netmodel::Family,
    input: ip_method::Input<'_>,
) -> io::Result<()> {
    let input = iface::Input::Method { family, input };
    feedback::interface_input(driver, now, wall, name, input).await
}
