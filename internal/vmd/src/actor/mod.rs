mod commands;
mod state;

use tokio::sync::{mpsc, oneshot};

use crate::clients::NetworkClient;
use crate::proto::vm::{VmConfig, VmInfo};

pub use commands::VmCommand;
use state::VmActor;

#[derive(Clone)]
pub struct VmActorHandle {
    tx: mpsc::Sender<VmCommand>,
}

impl VmActorHandle {
    pub async fn create(&self, config: VmConfig) -> anyhow::Result<String> {
        let (reply, rx) = oneshot::channel();
        self.tx.send(VmCommand::Create { config, reply }).await?;
        rx.await?
    }

    pub async fn start(&self, vm_id: String) -> anyhow::Result<()> {
        let (reply, rx) = oneshot::channel();
        self.tx.send(VmCommand::Start { vm_id, reply }).await?;
        rx.await?
    }

    pub async fn stop(&self, vm_id: String, force: bool) -> anyhow::Result<()> {
        let (reply, rx) = oneshot::channel();
        self.tx
            .send(VmCommand::Stop {
                vm_id,
                force,
                reply,
            })
            .await?;
        rx.await?
    }

    pub async fn delete(&self, vm_id: String) -> anyhow::Result<()> {
        let (reply, rx) = oneshot::channel();
        self.tx.send(VmCommand::Delete { vm_id, reply }).await?;
        rx.await?
    }

    pub async fn get(&self, vm_id: String) -> anyhow::Result<VmInfo> {
        let (reply, rx) = oneshot::channel();
        self.tx.send(VmCommand::Get { vm_id, reply }).await?;
        rx.await?
    }

    pub async fn list(&self) -> anyhow::Result<Vec<VmInfo>> {
        let (reply, rx) = oneshot::channel();
        self.tx.send(VmCommand::List { reply }).await?;
        rx.await?
    }

    pub async fn upload_file(
        &self,
        filename: String,
        data: Vec<u8>,
        vm_id: Option<String>,
    ) -> anyhow::Result<String> {
        let (reply, rx) = oneshot::channel();
        self.tx
            .send(VmCommand::UploadFile {
                filename,
                data,
                vm_id,
                reply,
            })
            .await?;
        rx.await?
    }

    pub async fn get_serial_log(&self, vm_id: String, tail_lines: i64) -> anyhow::Result<String> {
        let (reply, rx) = oneshot::channel();
        self.tx
            .send(VmCommand::GetSerialLog {
                vm_id,
                tail_lines,
                reply,
            })
            .await?;
        rx.await?
    }
}

pub async fn start_vm_actor(network_client: NetworkClient) -> VmActorHandle {
    let (cmd_tx, cmd_rx) = mpsc::channel(32);

    tokio::spawn(async move {
        let mut actor = VmActor::new(network_client);
        actor.run(cmd_rx).await;
    });

    VmActorHandle { tx: cmd_tx }
}
