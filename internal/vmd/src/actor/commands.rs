use tokio::sync::oneshot;

use crate::proto::vm::{VmConfig, VmInfo};

pub enum VmCommand {
    Create {
        config: VmConfig,
        reply: oneshot::Sender<anyhow::Result<String>>,
    },
    Start {
        vm_id: String,
        reply: oneshot::Sender<anyhow::Result<()>>,
    },
    Stop {
        vm_id: String,
        force: bool,
        reply: oneshot::Sender<anyhow::Result<()>>,
    },
    Delete {
        vm_id: String,
        reply: oneshot::Sender<anyhow::Result<()>>,
    },
    Get {
        vm_id: String,
        reply: oneshot::Sender<anyhow::Result<VmInfo>>,
    },
    List {
        reply: oneshot::Sender<anyhow::Result<Vec<VmInfo>>>,
    },
    UploadFile {
        filename: String,
        data: Vec<u8>,
        vm_id: Option<String>,
        reply: oneshot::Sender<anyhow::Result<String>>,
    },
    GetSerialLog {
        vm_id: String,
        tail_lines: i64,
        reply: oneshot::Sender<anyhow::Result<String>>,
    },
}
