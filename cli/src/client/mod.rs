mod connector;
mod upload;

pub use upload::upload_file;

use anyhow::{Context, Result, bail};
use tonic::transport::{Certificate, Channel, ClientTlsConfig, Endpoint, Identity};

use crate::config::ServerContext;
use connector::InsecureTlsConnector;

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

/// Connects using a server context with mTLS.
pub async fn connect(ctx: &ServerContext, timeout_secs: u64) -> Result<Channel> {
    let connect_timeout = std::time::Duration::from_secs(5);
    let request_timeout = std::time::Duration::from_secs(timeout_secs);

    let Some((ca, crt, key)) = ctx.credentials().context("Missing client credentials")? else {
        bail!("Client credentials are required for this operation");
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

/// Connects via TLS without verifying server certificate (TOFU model).
pub async fn connect_tls_insecure(server: &str, timeout_secs: u64) -> Result<Channel> {
    let connect_timeout = std::time::Duration::from_secs(5);
    let request_timeout = std::time::Duration::from_secs(timeout_secs);

    let connector = InsecureTlsConnector::new(server)?;

    // Use http:// scheme to bypass tonic's TLS check - our connector handles TLS internally
    let endpoint = Endpoint::from_shared(format!("http://{server}"))
        .context("Invalid server address")?
        .connect_timeout(connect_timeout)
        .timeout(request_timeout);

    endpoint
        .connect_with_connector(connector)
        .await
        .with_context(|| format!("Failed to connect to {} (TLS)", server))
}
