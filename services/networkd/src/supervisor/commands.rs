//! Commands accepted by the network supervisor.

use anyhow::Result;
use tokio::sync::oneshot;

#[derive(Debug)]
pub enum SupervisorCommand {
    Initialize { reply: oneshot::Sender<Result<()>> },
}
