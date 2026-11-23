//! Execution of supervisor effects against the runtime ports.

use alloc::collections::VecDeque;
use std::io;

use net_runtime::Link as _;

use crate::pending::Pending;

/// Executes one supervisor effect and queues the results the supervisor
/// must consume later.
pub(crate) async fn apply<R: net_runtime::Runtime>(
    runtime: &R,
    pending: &mut VecDeque<Pending>,
    effect: net_supervisor::Effect<'_>,
) -> io::Result<()> {
    match effect {
        net_supervisor::Effect::Discover => {
            let links = runtime.link().links().await?;
            pending.push_back(Pending::Links(links));
        }
        net_supervisor::Effect::ProbeCarriers { links } => {
            let carriers = links
                .iter()
                .map(|link| (link.index, link.has_carrier()))
                .collect();
            pending.push_back(Pending::Carriers(carriers));
        }
        net_supervisor::Effect::ConfigureInterface { name, v4, v6 } => {
            pending.push_back(Pending::ConfigureInterface {
                name,
                v4: v4.cloned(),
                v6: v6.cloned(),
            });
        }
        net_supervisor::Effect::EnsureBridge { name, config } => {
            runtime.link().ensure_bridge(&name, config.stp).await?;
        }
        net_supervisor::Effect::Enslave { port, master } => {
            runtime.link().set_master(&port, &master).await?;
        }
        net_supervisor::Effect::Detach { port } => {
            runtime.link().detach(&port).await?;
        }
        net_supervisor::Effect::ShutdownInterface { name } => {
            pending.push_back(Pending::ShutdownInterface { name });
        }
        // Publication belongs to the service that owns the status channel, and
        // the resolver file belongs to the service that owns name resolution.
        net_supervisor::Effect::Publish | net_supervisor::Effect::WriteResolver { .. } => {}
    }

    // TODO(Wave 3): the driver does not feed the supervisor's feedback inputs
    // yet. `Enslaved`, `Detached`, `BridgeReady`, `InterfaceSnapshot`,
    // `LinkEvent`, and `EffectFailed` need the event port, snapshot
    // publication, and effect error reporting that this skeleton lacks, so the
    // supervisor currently only ever sees poll, links, and carriers.
    Ok(())
}
