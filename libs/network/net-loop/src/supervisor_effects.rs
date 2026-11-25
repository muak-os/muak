//! Execution of supervisor effects against the runtime ports.

use alloc::collections::VecDeque;
use std::io;

use net_runtime::Link as _;

use crate::driver;
use crate::pending::Pending;
use crate::resolver::Nameservers;

/// Executes one supervisor effect and queues the results the supervisor
/// must consume later.
pub(crate) async fn apply<R: net_runtime::Runtime>(
    runtime: &R,
    pending: &mut VecDeque<Pending>,
    resolver: &mut Nameservers,
    effect: net_supervisor::Effect<'_>,
) -> io::Result<()> {
    match effect {
        net_supervisor::Effect::Discover => {
            let links = runtime.link().links().await?;
            pending.push_back(Pending::Links(links));
        }
        net_supervisor::Effect::ProbeCarriers => {
            let links = runtime.link().links().await?;
            let carriers = links
                .iter()
                .map(|link| (link.index, link.has_carrier()))
                .collect();
            pending.push_back(Pending::Carriers(carriers));
        }
        net_supervisor::Effect::SetLinkUp { name } => {
            runtime.link().set_up(&name).await?;
            // net-runtime-linux seeds its previous-state map on the first
            // events().next() after activation, so the initial RTM_NEWLINK can
            // be suppressed. Refresh the controller view here instead of
            // leaving it on a stale Down snapshot until the next event.
            if let Some(link) = runtime.link().link(&name).await? {
                pending.push_back(Pending::InterfaceLink {
                    name,
                    info: driver::link_info(&link),
                });
            }
        }
        net_supervisor::Effect::ConfigureInterface { name, v4, v6 } => {
            runtime.link().set_up(&name).await?;
            pending.push_back(Pending::ConfigureInterface {
                name,
                v4: v4.cloned(),
                v6: v6.cloned(),
            });
        }
        net_supervisor::Effect::ReconcileInterface { name } => {
            runtime.link().set_up(&name).await?;
            pending.push_back(Pending::ReconcileInterface { name });
        }
        net_supervisor::Effect::EnsureBridge { name, config } => {
            runtime.link().ensure_bridge(&name, config.stp).await?;
            runtime.link().set_up(&name).await?;
            let Some(link) = runtime.link().link(&name).await? else {
                return Err(io::Error::new(
                    io::ErrorKind::NotFound,
                    "bridge link is missing after creation",
                ));
            };
            pending.push_back(Pending::BridgeReady {
                name,
                index: link.index,
                mac: link.mac,
                state: link.state,
            });
        }
        net_supervisor::Effect::Enslave { port, master } => {
            runtime.link().set_up(&port).await?;
            runtime.link().set_master(&port, &master).await?;
            pending.push_back(Pending::Enslaved { port, master });
        }
        net_supervisor::Effect::Detach { port } => {
            runtime.link().detach(&port).await?;
            pending.push_back(Pending::Detached { port });
        }
        net_supervisor::Effect::ShutdownInterface { name } => {
            pending.push_back(Pending::ShutdownInterface { name });
        }
        net_supervisor::Effect::Publish => {}
        net_supervisor::Effect::WriteResolver { v4, v6 } => resolver.store(v4, v6),
    }

    Ok(())
}
