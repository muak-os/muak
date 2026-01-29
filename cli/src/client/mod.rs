mod upload;

pub use upload::upload_file;

use anyhow::{Context, Result, bail};
use std::path::PathBuf;
use tonic::transport::{Certificate, Channel, ClientTlsConfig, Identity};

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
    ApproveCsrRequest, ListPendingCsrsRequest, ListUsersRequest, RevokeCertRequest,
};

pub const CA_CERT_FILE: &str = "ca.crt";
pub const CLIENT_CERT_FILE: &str = "client.crt";
pub const CLIENT_KEY_FILE: &str = "client.key";

/// Gets the muak config directory
pub fn config_dir() -> Result<PathBuf> {
    let home = std::env::var("HOME").context("HOME environment variable not set")?;
    Ok(PathBuf::from(home).join(".config/muak"))
}

/// Checks if mTLS credentials are fully configured
pub fn has_credentials() -> bool {
    if let Ok(dir) = config_dir() {
        dir.join(CA_CERT_FILE).exists()
            && dir.join(CLIENT_CERT_FILE).exists()
            && dir.join(CLIENT_KEY_FILE).exists()
    } else {
        false
    }
}

/// Loads mTLS credentials from config dir
fn load_tls_credentials() -> Result<(Certificate, Identity)> {
    let dir = config_dir()?;

    let ca_path = dir.join(CA_CERT_FILE);
    let cert_path = dir.join(CLIENT_CERT_FILE);
    let key_path = dir.join(CLIENT_KEY_FILE);

    if !ca_path.exists() || !cert_path.exists() || !key_path.exists() {
        bail!("mTLS credentials not found. Run 'muakctl install' to set up credentials.");
    }

    let ca_pem = std::fs::read(&ca_path)
        .with_context(|| format!("Failed to read CA certificate from {:?}", ca_path))?;
    let cert_pem = std::fs::read(&cert_path)
        .with_context(|| format!("Failed to read client certificate from {:?}", cert_path))?;
    let key_pem = std::fs::read(&key_path)
        .with_context(|| format!("Failed to read client key from {:?}", key_path))?;

    let ca = Certificate::from_pem(ca_pem);
    let identity = Identity::from_pem(cert_pem, key_pem);

    Ok((ca, identity))
}

/// Creates a gRPC channel with appropriate TLS configuration.
///
/// Connection strategy:
/// - If credentials exist: use mTLS, fall back to HTTP if TLS handshake fails (maintenance mode)
/// - If no credentials: try HTTP (maintenance mode only)
pub async fn connect(server: &str, timeout_secs: u64) -> Result<Channel> {
    let connect_timeout = std::time::Duration::from_secs(5);
    let request_timeout = std::time::Duration::from_secs(timeout_secs);

    if let Ok((ca, identity)) = load_tls_credentials() {
        // We have credentials - try mTLS first
        let tls_config = ClientTlsConfig::new()
            .ca_certificate(ca)
            .identity(identity)
            .domain_name("muak-server");

        if let Ok(endpoint) = Channel::from_shared(format!("https://{server}")) {
            if let Ok(configured) = endpoint.tls_config(tls_config) {
                match configured
                    .connect_timeout(connect_timeout)
                    .timeout(request_timeout)
                    .connect()
                    .await
                {
                    Ok(ch) => return Ok(ch),
                    Err(_) => {
                        // mTLS failed - server might be in maintenance mode, try HTTP
                    }
                }
            }
        }

        // Fall back to HTTP (maintenance mode with stale credentials)
        return Channel::from_shared(format!("http://{server}"))?
            .connect_timeout(connect_timeout)
            .timeout(request_timeout)
            .connect()
            .await
            .context("Failed to connect to server");
    }

    // No credentials - can only connect to maintenance mode (HTTP)
    Channel::from_shared(format!("http://{server}"))?
        .connect_timeout(connect_timeout)
        .timeout(request_timeout)
        .connect()
        .await
        .context(
            "Failed to connect to server. \
            If the server is already installed, you need credentials from the administrator.",
        )
}

/// Saves credentials to config dir
pub fn save_credentials(ca_pem: &str, cert_pem: &str) -> Result<()> {
    let dir = config_dir()?;
    std::fs::create_dir_all(&dir)?;

    std::fs::write(dir.join(CA_CERT_FILE), ca_pem)?;
    std::fs::write(dir.join(CLIENT_CERT_FILE), cert_pem)?;

    Ok(())
}
