//! Authentication commands for certificate management (admin only).

use core::time::Duration;

use anyhow::{Context as _, Result};
use clap::Subcommand;
use config::user::ClientConfig;
use pki::csr;
use tokio::time::sleep;
use tonic::transport::Channel;
use tui::style::Role;

use crate::client::{
    auth_service::{
        AckEnrollmentRequest, ApproveCsrRequest, GetCsrStatusRequest, ListPendingCsrsRequest,
        ListUsersRequest, RevokeCertRequest, SubmitCsrRequest,
        auth_service_client::AuthServiceClient, get_csr_status_response::Status as CsrStatus,
    },
    connect_tls_insecure, connect_tls_pinned,
};
use crate::console::Console;

#[derive(Subcommand, Clone)]
pub enum Action {
    Requests,
    Approve {
        fingerprint: String,
        #[arg(long, default_value = "system:read")]
        permissions: String,
    },
    Revoke {
        fingerprint: String,
    },
    List,
}

/// Handles authentication commands.
pub async fn handle(channel: Channel, action: Action, console: &Console) -> Result<()> {
    match action {
        Action::Requests => requests(channel, console).await,
        Action::Approve {
            fingerprint,
            permissions,
        } => approve(channel, &fingerprint, &permissions, console).await,
        Action::Revoke { fingerprint } => revoke(channel, &fingerprint, console).await,
        Action::List => list(channel, console).await,
    }
}

/// Lists pending authentication requests (admin only).
async fn requests(channel: Channel, console: &Console) -> Result<()> {
    let mut auth_client = AuthServiceClient::new(channel);

    let response = auth_client
        .list_pending_csrs(ListPendingCsrsRequest {})
        .await
        .context("Failed to list pending CSRs")?;

    let csrs = response.into_inner().csrs;

    if csrs.is_empty() {
        println!("No pending authentication requests.");
        return Ok(());
    }

    let count = csrs.len().to_string();
    println!(
        "{} pending authentication request(s):\n",
        console.paint(&count, Role::Accent)
    );

    for csr in csrs {
        let display_fp = short_fingerprint(&csr.fingerprint);
        println!(
            "  {} (submitted: {})",
            console.paint(display_fp, Role::Highlight),
            csr.submitted_at
        );
    }

    println!(
        "\nTo approve: {} <fingerprint> --permissions admin",
        console.paint("muakctl auth approve", Role::Positive)
    );

    Ok(())
}

/// Approves a pending authentication request (admin only).
async fn approve(
    channel: Channel,
    fingerprint: &str,
    permissions: &str,
    console: &Console,
) -> Result<()> {
    let mut auth_client = AuthServiceClient::new(channel);

    let perms: Vec<String> = permissions
        .split(',')
        .map(|perm| perm.trim().to_owned())
        .collect();

    let pending_response = auth_client
        .list_pending_csrs(ListPendingCsrsRequest {})
        .await
        .context("Failed to list pending CSRs")?;

    let csrs = pending_response.into_inner().csrs;
    let matching: Vec<_> = csrs
        .iter()
        .filter(|csr| csr.fingerprint.starts_with(fingerprint))
        .collect();

    let full_fingerprint = match matching.len() {
        0 => {
            console.error(&format!("No pending CSR matches '{fingerprint}'"));
            return Ok(());
        }
        1 => matching
            .first()
            .ok_or_else(|| anyhow::anyhow!("no matching CSR"))?
            .fingerprint
            .clone(),
        _ => {
            console.error(&format!(
                "Multiple CSRs match '{fingerprint}'. Please be more specific:"
            ));
            for csr in matching {
                println!("  {}", csr.fingerprint);
            }
            return Ok(());
        }
    };

    let short_fp = short_fingerprint(&full_fingerprint).to_owned();
    println!(
        "Approving CSR: {}",
        console.paint(&short_fp, Role::Highlight)
    );
    println!("Permissions: {perms:?}");

    auth_client
        .approve_csr(ApproveCsrRequest {
            fingerprint: full_fingerprint.clone(),
            permissions: perms,
        })
        .await
        .context("Failed to approve CSR")?;

    println!();
    console.success("Certificate issued!");

    Ok(())
}

/// Revokes a certificate (admin only).
async fn revoke(channel: Channel, fingerprint: &str, console: &Console) -> Result<()> {
    let mut auth_client = AuthServiceClient::new(channel);

    let users_response = auth_client
        .list_users(ListUsersRequest {})
        .await
        .context("Failed to list users")?;

    let users = users_response.into_inner().users;
    let matching: Vec<_> = users
        .iter()
        .filter(|user| user.fingerprint.starts_with(fingerprint))
        .collect();

    let full_fingerprint = match matching.len() {
        0 => {
            console.error(&format!("No user matches fingerprint '{fingerprint}'"));
            return Ok(());
        }
        1 => matching
            .first()
            .ok_or_else(|| anyhow::anyhow!("no matching user"))?
            .fingerprint
            .clone(),
        _ => {
            console.error(&format!(
                "Multiple users match '{fingerprint}'. Please be more specific:"
            ));
            for user in matching {
                println!("  {}", user.fingerprint);
            }
            return Ok(());
        }
    };

    let short_fp = short_fingerprint(&full_fingerprint).to_owned();
    println!(
        "Revoking certificate: {}",
        console.paint(&short_fp, Role::Highlight)
    );

    auth_client
        .revoke_cert(RevokeCertRequest {
            fingerprint: full_fingerprint.clone(),
        })
        .await
        .context("Failed to revoke certificate")?;

    console.success("Certificate revoked.");

    Ok(())
}

/// Lists all authorized users (admin only).
async fn list(channel: Channel, console: &Console) -> Result<()> {
    let mut auth_client = AuthServiceClient::new(channel);

    let response = auth_client
        .list_users(ListUsersRequest {})
        .await
        .context("Failed to list users")?;

    let inner = response.into_inner();

    if inner.users.is_empty() {
        println!("No authorized users.");
    } else {
        let count = inner.users.len().to_string();
        println!(
            "{} authorized user(s):\n",
            console.paint(&count, Role::Accent)
        );

        for user in &inner.users {
            let display_fp = short_fingerprint(&user.fingerprint);
            let perms = user.permissions.join(", ");
            println!(
                "  {} [{}]",
                console.paint(display_fp, Role::Positive),
                console.paint(&perms, Role::Muted)
            );
        }
    }

    if !inner.revoked_fingerprints.is_empty() {
        let count = inner.revoked_fingerprints.len().to_string();
        println!(
            "\n{} revoked certificate(s):",
            console.paint(&count, Role::Negative)
        );
        for fp in &inner.revoked_fingerprints {
            let display_fp = short_fingerprint(fp);
            println!("  {}", console.paint(display_fp, Role::Muted));
        }
    }

    Ok(())
}

/// Enrolls with a server by generating a CSR and polling for approval.
pub async fn enroll(endpoint: &str, console: &Console) -> Result<()> {
    let mut config = ClientConfig::load()?;

    if config.has_credentials_for_endpoint(endpoint) {
        println!("Already authenticated to {endpoint}.");
        println!(
            "Use '{}' to re-enroll.",
            console.paint("muakctl context remove <name>", Role::Accent)
        );
        return Ok(());
    }

    let (fingerprint, key_pem, server_fingerprint) =
        if let Some(pending) = config.get_pending(endpoint) {
            println!(
                "Resuming pending enrollment for {}",
                console.paint(endpoint, Role::Accent)
            );
            let key_pem = pending.key_pem()?;
            (
                pending.fingerprint.clone(),
                key_pem,
                pending.server_fingerprint.clone(),
            )
        } else {
            println!("Generating key pair...");
            let (key_pem, csr_pem) = csr::generate("muak-client")?;
            let fingerprint = csr::compute_fingerprint(&csr_pem)?;

            println!("Submitting certificate request...");
            let (channel, server_fp) = connect_tls_insecure(endpoint, 30).await?;
            let mut client = AuthServiceClient::new(channel);

            client
                .submit_csr(SubmitCsrRequest { csr_pem })
                .await
                .context("Failed to submit CSR")?;

            config.start_enrollment(endpoint, &key_pem, &fingerprint, &server_fp);
            config.save()?;

            (fingerprint, key_pem, server_fp)
        };

    let display_fp = short_fingerprint(&fingerprint);
    println!(
        "\nFingerprint: {}",
        console.paint(display_fp, Role::Highlight)
    );
    println!(
        "Ask an admin to run: {} {}...",
        console.paint("muakctl auth approve", Role::Positive),
        display_fp
    );

    wait_for_approval(
        endpoint,
        &fingerprint,
        &key_pem,
        &server_fingerprint,
        console,
    )
    .await
}

/// Polls the server until the CSR is approved, rejected, or no longer found.
async fn wait_for_approval(
    endpoint: &str,
    fingerprint: &str,
    key_pem: &str,
    server_fingerprint: &str,
    console: &Console,
) -> Result<()> {
    let spinner = console.spinner("Waiting for admin approval... (Ctrl+C to resume later)")?;

    let channel = connect_tls_pinned(endpoint, 30, server_fingerprint).await?;
    let mut client = AuthServiceClient::new(channel);

    loop {
        let response = client
            .get_csr_status(GetCsrStatusRequest {
                fingerprint: fingerprint.to_owned(),
            })
            .await
            .context("Failed to check CSR status")?;

        let status = response.into_inner();
        match status.status() {
            CsrStatus::Pending => {}
            CsrStatus::Approved => {
                spinner.success("Approved!")?;

                let mut config = ClientConfig::load()?;
                let name = config.complete_enrollment(
                    endpoint,
                    &status.server_name,
                    &status.ca_pem,
                    &status.cert_pem,
                    key_pem.as_bytes(),
                );
                config.save()?;

                let _ack_result = client
                    .ack_enrollment(AckEnrollmentRequest {
                        fingerprint: fingerprint.to_owned(),
                    })
                    .await;

                console.line(
                    Role::Accent,
                    &format!("Context '{name}' created and set as current."),
                );
                return Ok(());
            }
            CsrStatus::Rejected => {
                spinner.fail("Request was rejected by admin.")?;
                let mut config = ClientConfig::load()?;
                config.cancel_enrollment(endpoint);
                config.save()?;
                return Ok(());
            }
            CsrStatus::NotFound => {
                spinner.fail("CSR not found on server (may have expired).")?;
                println!("Run the command again to submit a new request.");
                let mut config = ClientConfig::load()?;
                config.cancel_enrollment(endpoint);
                config.save()?;
                return Ok(());
            }
        }

        sleep(Duration::from_secs(3)).await;
    }
}

fn short_fingerprint(fp: &str) -> &str {
    fp.get(..16).unwrap_or(fp)
}
