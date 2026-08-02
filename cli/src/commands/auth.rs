//! Authentication commands for certificate management (admin only).

use core::time::Duration;

use anyhow::{Context as _, Result};
use base64ct::{Base64, Encoding as _};
use clap::Subcommand;
use config::ClientConfig;
use pki::csr;
use tokio::time::sleep;
use tonic::transport::Channel;

use crate::client::{
    auth_service::{
        AckEnrollmentRequest, ApproveCsrRequest, GetCsrStatusRequest, ListPendingCsrsRequest,
        ListUsersRequest, RevokeCertRequest, SubmitCsrRequest,
        auth_service_client::AuthServiceClient, get_csr_status_response::Status as CsrStatus,
    },
    connect_tls_insecure, connect_tls_pinned,
};
use crate::ui;

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
pub async fn handle(channel: Channel, action: Action) -> Result<()> {
    match action {
        Action::Requests => requests(channel).await,
        Action::Approve {
            fingerprint,
            permissions,
        } => approve(channel, &fingerprint, &permissions).await,
        Action::Revoke { fingerprint } => revoke(channel, &fingerprint).await,
        Action::List => list(channel).await,
    }
}

/// Lists pending authentication requests (admin only).
async fn requests(channel: Channel) -> Result<()> {
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
        ui::style::accent(&count)
    );

    for csr in csrs {
        let display_fp = short_fingerprint(&csr.fingerprint);
        println!(
            "  {} (submitted: {})",
            ui::style::highlight(display_fp),
            csr.submitted_at
        );
    }

    println!(
        "\nTo approve: {} <fingerprint> --permissions admin",
        ui::style::positive("muakctl auth approve")
    );

    Ok(())
}

/// Approves a pending authentication request (admin only).
async fn approve(channel: Channel, fingerprint: &str, permissions: &str) -> Result<()> {
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
            println!(
                "{}: No pending CSR matches '{fingerprint}'",
                ui::style::error("Error")
            );
            return Ok(());
        }
        1 => matching
            .first()
            .ok_or_else(|| anyhow::anyhow!("no matching CSR"))?
            .fingerprint
            .clone(),
        _ => {
            println!(
                "{}: Multiple CSRs match '{fingerprint}'. Please be more specific:",
                ui::style::error("Error")
            );
            for csr in matching {
                println!("  {}", csr.fingerprint);
            }
            return Ok(());
        }
    };

    let short_fp = short_fingerprint(&full_fingerprint).to_owned();
    println!("Approving CSR: {}", ui::style::highlight(&short_fp));
    println!("Permissions: {perms:?}");

    auth_client
        .approve_csr(ApproveCsrRequest {
            fingerprint: full_fingerprint.clone(),
            permissions: perms,
        })
        .await
        .context("Failed to approve CSR")?;

    println!("\n{} Certificate issued!", ui::style::success("Success:"));

    Ok(())
}

/// Revokes a certificate (admin only).
async fn revoke(channel: Channel, fingerprint: &str) -> Result<()> {
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
            println!(
                "{}: No user matches fingerprint '{fingerprint}'",
                ui::style::error("Error")
            );
            return Ok(());
        }
        1 => matching
            .first()
            .ok_or_else(|| anyhow::anyhow!("no matching user"))?
            .fingerprint
            .clone(),
        _ => {
            println!(
                "{}: Multiple users match '{fingerprint}'. Please be more specific:",
                ui::style::error("Error")
            );
            for user in matching {
                println!("  {}", user.fingerprint);
            }
            return Ok(());
        }
    };

    let short_fp = short_fingerprint(&full_fingerprint).to_owned();
    println!("Revoking certificate: {}", ui::style::highlight(&short_fp));

    auth_client
        .revoke_cert(RevokeCertRequest {
            fingerprint: full_fingerprint.clone(),
        })
        .await
        .context("Failed to revoke certificate")?;

    println!("{} Certificate revoked.", ui::style::success("Success:"));

    Ok(())
}

/// Lists all authorized users (admin only).
async fn list(channel: Channel) -> Result<()> {
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
        println!("{} authorized user(s):\n", ui::style::accent(&count));

        for user in &inner.users {
            let display_fp = short_fingerprint(&user.fingerprint);
            let perms = user.permissions.join(", ");
            println!(
                "  {} [{}]",
                ui::style::positive(display_fp),
                ui::style::muted(&perms)
            );
        }
    }

    if !inner.revoked_fingerprints.is_empty() {
        let count = inner.revoked_fingerprints.len().to_string();
        println!("\n{} revoked certificate(s):", ui::style::negative(&count));
        for fp in &inner.revoked_fingerprints {
            let display_fp = short_fingerprint(fp);
            println!("  {}", ui::style::muted(display_fp));
        }
    }

    Ok(())
}

/// Enrolls with a server by generating a CSR and polling for approval.
pub async fn enroll(endpoint: &str) -> Result<()> {
    let mut config = ClientConfig::load()?;

    if config.has_credentials_for_endpoint(endpoint) {
        println!("Already authenticated to {endpoint}.");
        println!(
            "Use '{}' to re-enroll.",
            ui::style::accent("muakctl context remove <name>")
        );
        return Ok(());
    }

    let (fingerprint, key_pem, server_fingerprint) =
        if let Some(pending) = config.get_pending(endpoint) {
            println!(
                "Resuming pending enrollment for {}",
                ui::style::accent(endpoint)
            );
            let key = Base64::decode_vec(&pending.key).context("Failed to decode pending key")?;
            let key_pem = String::from_utf8(key).context("Invalid key encoding")?;
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
    println!("\nFingerprint: {}", ui::style::highlight(display_fp));
    println!(
        "Ask an admin to run: {} {}...",
        ui::style::positive("muakctl auth approve"),
        display_fp
    );

    wait_for_approval(endpoint, &fingerprint, &key_pem, &server_fingerprint).await
}

/// Polls the server until the CSR is approved, rejected, or no longer found.
async fn wait_for_approval(
    endpoint: &str,
    fingerprint: &str,
    key_pem: &str,
    server_fingerprint: &str,
) -> Result<()> {
    let spinner =
        ui::spinner::Spinner::start("Waiting for admin approval... (Ctrl+C to resume later)");

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
                spinner.success("Approved!").await;

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

                println!(
                    "Context '{}' created and set as current.",
                    ui::style::accent(&name)
                );
                return Ok(());
            }
            CsrStatus::Rejected => {
                spinner.fail("Request was rejected by admin.").await;
                let mut config = ClientConfig::load()?;
                config.cancel_enrollment(endpoint);
                config.save()?;
                return Ok(());
            }
            CsrStatus::NotFound => {
                spinner
                    .fail("CSR not found on server (may have expired).")
                    .await;
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
