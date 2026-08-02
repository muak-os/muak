//! Prints client and server version with compatibility status.

use anyhow::Result;
use config::{CompatibilityStatus, parse_pkg_version};
use tonic::transport::Channel;

use crate::client::version_service::{
    GetVersionRequest, version_service_client::VersionServiceClient,
};
use crate::ui;

/// Fetches the server version string via gRPC.
pub async fn fetch_server(channel: Channel) -> Option<String> {
    let mut client = VersionServiceClient::new(channel);
    client
        .get_version(GetVersionRequest {})
        .await
        .ok()
        .map(|resp| resp.into_inner().version)
}

/// Handles the `version` command: prints client/server versions and any warnings.
pub async fn handle(channel: Channel) -> Result<()> {
    let client_ver = env!("CARGO_PKG_VERSION");
    println!("{} {}", ui::style::label("Client:"), client_ver);

    match fetch_server(channel).await {
        None => {
            eprintln!(
                "{} server does not support version reporting (consider updating the server)",
                ui::style::warn("Warning:")
            );
        }
        Some(ref server_ver) => {
            println!("{} {}", ui::style::label("Server:"), server_ver);
            print_compat_warning(client_ver, server_ver);
        }
    }

    Ok(())
}

/// Prints a warning to stderr if versions are not identical.
pub fn print_compat_warning(client_ver: &str, server_ver: &str) {
    if client_ver == server_ver {
        return;
    }

    let (Ok(cli), Ok(srv)) = (parse_pkg_version(client_ver), parse_pkg_version(server_ver)) else {
        return;
    };

    let msg = match config::check_compatibility(&cli, &srv) {
        CompatibilityStatus::Compatible => return,
        CompatibilityStatus::MinorDrift { cli_newer: true } => {
            format!(
                "server version {server_ver} is older than client {client_ver}; consider updating the server"
            )
        }
        CompatibilityStatus::MinorDrift { cli_newer: false } => {
            format!(
                "client version {client_ver} is older than server {server_ver}; consider updating muakctl"
            )
        }
        CompatibilityStatus::MajorMismatch { cli_newer: true } => {
            format!(
                "significant version mismatch: server {server_ver} vs client {client_ver} \
                 (major version differs) — API compatibility not guaranteed; \
                 consider updating the server"
            )
        }
        CompatibilityStatus::MajorMismatch { cli_newer: false } => {
            format!(
                "significant version mismatch: client {client_ver} vs server {server_ver} \
                 (major version differs) — API compatibility not guaranteed; \
                 consider updating muakctl"
            )
        }
    };

    eprintln!("{} {}", ui::style::warn("Warning:"), msg);
}
