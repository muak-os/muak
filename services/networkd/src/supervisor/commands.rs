//! Commands accepted by the network supervisor.

use anyhow::Result;
use tokio::sync::oneshot;

use super::Status;

#[derive(Debug)]
pub(super) enum SupervisorCommand {
    Initialize { reply: oneshot::Sender<Result<()>> },
    Reconcile { reply: oneshot::Sender<()> },
    Status { reply: oneshot::Sender<Status> },
}
