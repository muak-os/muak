use anyhow::Result;
use tokio::sync::{mpsc, oneshot};

use super::state::NetworkActor;
use crate::model::{ConnectivityResult, InterfaceSnapshot, NetworkSnapshot};
use crate::slaac::SlaacEvent;

#[derive(Debug)]
#[allow(dead_code)]
pub enum NetworkCommand {
    Initialize {
        reply: oneshot::Sender<Result<()>>,
    },
    SetupBridge {
        reply: oneshot::Sender<Result<()>>,
    },
    AddTap {
        name: String,
        reply: oneshot::Sender<Result<InterfaceSnapshot>>,
    },
    DeleteTap {
        name: String,
        reply: oneshot::Sender<Result<()>>,
    },
    AcquireDhcp {
        iface: String,
        reply: oneshot::Sender<Result<InterfaceSnapshot>>,
    },
    // Internal command triggered by timer
    RenewLease {
        iface: String,
    },
    Snapshot {
        reply: oneshot::Sender<NetworkSnapshot>,
    },
    CheckConnectivity {
        reply: oneshot::Sender<ConnectivityResult>,
    },
    Slaac(SlaacEvent),
    // Internal periodic connectivity check trigger
    PeriodicConnectivityCheck,
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
            NetworkCommand::SetupBridge { reply } => {
                let result = self.setup_bridge(cmd_tx).await;
                let _ = reply.send(result);
            }
            NetworkCommand::AddTap { name, reply } => {
                let result = self.add_tap(&name).await;
                let _ = reply.send(result);
            }
            NetworkCommand::DeleteTap { name, reply } => {
                let result = self.delete_tap(&name).await;
                let _ = reply.send(result);
            }
            NetworkCommand::AcquireDhcp { iface, reply } => {
                let result = self.acquire_dhcp(&iface, cmd_tx).await;
                let _ = reply.send(result);
            }
            NetworkCommand::RenewLease { iface } => {
                let _ = self.renew_lease(&iface).await;
            }
            NetworkCommand::Snapshot { reply } => {
                let _ = reply.send(self.state.clone());
            }
            NetworkCommand::CheckConnectivity { reply } => {
                let result = self.check_connectivity().await;
                let _ = reply.send(result);
            }
            NetworkCommand::PeriodicConnectivityCheck => {
                let _ = self.check_connectivity().await;
            }
            NetworkCommand::Slaac(event) => {
                self.handle_slaac_event(event).await;
            }
        }
    }
}
