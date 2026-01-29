mod upload;

pub use upload::upload_file;

use anyhow::{Context, Result};
use tonic::transport::{Certificate, Channel, ClientTlsConfig, Identity};

use crate::config::ServerContext;

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

#[allow(clippy::excessive_nesting)]
pub mod auth_service {
    tonic::include_proto!("muak.auth.v1");
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

pub use auth_service::auth_service_client::AuthServiceClient;
pub use auth_service::{
    ApproveCsrRequest, GetCsrStatusRequest, ListPendingCsrsRequest, ListUsersRequest,
    RevokeCertRequest, SubmitCsrRequest, get_csr_status_response::Status as CsrStatus,
};

/// Connect using a server context with mTLS.
pub async fn connect(ctx: &ServerContext, timeout_secs: u64) -> Result<Channel> {
    let connect_timeout = std::time::Duration::from_secs(5);
    let request_timeout = std::time::Duration::from_secs(timeout_secs);

    let Some((ca, crt, key)) = ctx.credentials()? else {
        return connect_insecure(&ctx.endpoint, timeout_secs).await;
    };

    let ca = Certificate::from_pem(ca);
    let identity = Identity::from_pem(crt, key);

    let tls_config = ClientTlsConfig::new()
        .ca_certificate(ca)
        .identity(identity)
        .domain_name("muak-server");

    let endpoint = Channel::from_shared(format!("https://{}", ctx.endpoint))
        .context("Invalid endpoint")?
        .tls_config(tls_config)
        .context("Failed to configure TLS")?
        .connect_timeout(connect_timeout)
        .timeout(request_timeout);

    endpoint
        .connect()
        .await
        .with_context(|| format!("Failed to connect to {}", ctx.endpoint))
}

/// Connect in maintenance mode (plain HTTP, no mTLS).
pub async fn connect_insecure(server: &str, timeout_secs: u64) -> Result<Channel> {
    let connect_timeout = std::time::Duration::from_secs(5);
    let request_timeout = std::time::Duration::from_secs(timeout_secs);

    Channel::from_shared(format!("http://{server}"))
        .context("Invalid server address")?
        .connect_timeout(connect_timeout)
        .timeout(request_timeout)
        .connect()
        .await
        .with_context(|| {
            format!(
                "Failed to connect to {} (maintenance mode). Is the server running?",
                server
            )
        })
}
