//! Commands accepted by the network actor and their dispatch logic.

use anyhow::Result;
use tokio::sync::{mpsc, oneshot};

use super::state::{InterfaceSnapshot, NetworkActor};
use crate::slaac::SlaacEvent;

#[derive(Debug)]
#[allow(dead_code)]
pub enum NetworkCommand {
    Initialize {
        reply: oneshot::Sender<Result<()>>,
    },
    AcquireDhcp {
        iface: String,
        reply: oneshot::Sender<Result<InterfaceSnapshot>>,
    },
    RenewLease {
        iface: String,
    },
    Slaac(SlaacEvent),
}

impl NetworkActor {
    pub(super) async fn handle_command(
        &mut self,
        cmd: NetworkCommand,
        cmd_tx: &mpsc::Sender<NetworkCommand>,
    ) {
        match cmd {
            NetworkCommand::Initialize { reply } => {
                let result = self.initialize(cmd_tx).await;
                let _ = reply.send(result);
            }
            NetworkCommand::AcquireDhcp { iface, reply } => {
                let result = self.acquire_dhcp(&iface, cmd_tx).await;
                let _ = reply.send(result);
            }
            NetworkCommand::RenewLease { iface } => {
                let _ = self.renew_lease(&iface).await;
            }
            NetworkCommand::Slaac(event) => {
                self.handle_slaac_event(event).await;
            }
        }
    }
}
