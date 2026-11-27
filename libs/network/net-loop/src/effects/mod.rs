//! Execution of interface and supervisor effects against the runtime ports.

mod address;
mod socket;
pub(crate) mod supervisor;

use alloc::collections::VecDeque;

use net_observe::journal::Journal;
use net_runtime::{Entropy as _, Link as _};
pub(crate) use socket::release;

use crate::SocketTable;
use crate::error::Result;
use crate::pending::Pending;

/// Executes one interface effect with the owning interface name in scope.
pub(crate) async fn apply<R: net_runtime::Runtime>(
    runtime: &R,
    interface: &netmodel::Name,
    sockets: &mut SocketTable,
    pending: &mut VecDeque<Pending>,
    journal: &mut Journal,
    effect: iface::Effect<'_>,
) -> Result<()> {
    match effect {
        iface::Effect::SetLinkUp { link } => {
            runtime.link().set_up(&link).await?;

            Ok(())
        }
        iface::Effect::SetLinkDown { link } => {
            runtime.link().set_down(&link).await?;

            Ok(())
        }
        iface::Effect::AddAddress {
            link,
            address,
            prefix_len,
        } => address::add(runtime, journal, link, address, prefix_len).await,
        iface::Effect::RemoveAddress {
            link,
            address,
            prefix_len,
        } => address::remove(runtime, journal, link, address, prefix_len).await,
        iface::Effect::EnsureRoute { link, gateway } => {
            address::ensure_route(runtime, journal, link, gateway).await
        }
        iface::Effect::RemoveRoute { link, gateway } => {
            address::remove_route(runtime, journal, link, gateway).await
        }
        iface::Effect::OpenSocket { family, spec } => {
            socket::open(runtime, interface, sockets, pending, family, spec).await
        }
        iface::Effect::Send {
            family,
            slot,
            frame,
            dst,
        } => {
            let transmit = socket::Transmit {
                family,
                slot,
                frame,
                dst,
            };
            socket::send(runtime, interface, sockets, pending, transmit).await
        }
        iface::Effect::CloseSocket { family, slot } => {
            socket::close(runtime, interface, sockets, pending, family, slot).await
        }
        iface::Effect::RequestEntropy { family } => {
            let mut bytes = [0_u8; 4];
            runtime.entropy().fill(&mut bytes);
            pending.push_back(Pending::Entropy {
                name: interface.clone(),
                family,
                value: u32::from_le_bytes(bytes),
            });

            Ok(())
        }
        iface::Effect::Publish => Ok(()),
    }
}
