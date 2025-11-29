use anyhow::Result;
use tokio::sync::{mpsc, oneshot};

use crate::network::model::InterfaceSnapshot;

use super::state::NetworkActor;

#[derive(Debug)]
pub enum NetworkCommand {
    Initialize {
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
    // Internal command triggered by timer
    RenewLease {
        iface: String,
    },
    // Internal command for IPv6 lease renewal
    RenewLeaseV6 {
        iface: String,
    },
    // Internal command triggered by failover
    PromoteSecondary {
        secondary: String,
    },
    // Internal command triggered by recovery
    RecoverPrimary {
        from_secondary: String,
        to_primary: String,
    },
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
            NetworkCommand::AddTap { name, reply } => {
                let result = self.add_tap(&name).await;
                let _ = reply.send(result);
            }
            NetworkCommand::DeleteTap { name, reply } => {
                let result = self.delete_tap(&name).await;
                let _ = reply.send(result);
            }
            NetworkCommand::RenewLease { iface } => {
                let _ = self.renew_lease(&iface).await;
            }
            NetworkCommand::RenewLeaseV6 { iface } => {
                let _ = self.renew_lease_v6(&iface).await;
            }
            NetworkCommand::PromoteSecondary { secondary } => {
                let _ = self.promote_secondary(&secondary, cmd_tx).await;
            }
            NetworkCommand::RecoverPrimary {
                from_secondary,
                to_primary,
            } => {
                let _ = self.recover_primary(&from_secondary, &to_primary).await;
            }
        }
    }
}
