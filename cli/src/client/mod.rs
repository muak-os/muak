mod upload;

pub use upload::upload_file;

use anyhow::{Context, Result};
use tonic::transport::Channel;

#[allow(clippy::excessive_nesting)]
pub mod process_service {
    tonic::include_proto!("muak.process.v1");
}

#[allow(clippy::excessive_nesting)]
pub mod vm_service {
    tonic::include_proto!("muak.vm.v1");
}

#[allow(clippy::excessive_nesting)]
pub mod provision_service {
    tonic::include_proto!("muak.provision.v1");
}

pub use process_service::ListProcessesRequest;
pub use process_service::process_service_client::ProcessServiceClient;

pub use provision_service::provision_service_client::ProvisionServiceClient;
pub use provision_service::{
    GetConfigRequest, GetLogsRequest, GetUpdateStatusRequest, InstallRequest, ListDisksRequest,
    PrepareUpdateRequest, UpdateRequest, UpdateStatus,
};

pub use vm_service::vm_service_client::VmServiceClient;
pub use vm_service::{
    CreateVmRequest, DeleteVmRequest, DiskConfig, GetVmSerialLogRequest, Hypervisor,
    ListVmsRequest, StartVmRequest, StopVmRequest, UploadFileRequest, VmConfig, VmState,
};

/// Creates a gRPC channel with the specified timeout.
pub async fn connect(server: &str, timeout_secs: u64) -> Result<Channel> {
    Channel::from_shared(format!("http://{server}"))?
        .connect_timeout(std::time::Duration::from_secs(5))
        .timeout(std::time::Duration::from_secs(timeout_secs))
        .connect()
        .await
        .context("Failed to connect to server")
}
