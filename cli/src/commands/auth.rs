//! Authentication commands for certificate management (admin only).

use std::time::Duration;

use anyhow::{Context, Result};
use base64ct::{Base64, Encoding};
use clap::Subcommand;
use tonic::transport::Channel;

use crate::client::{
    AckEnrollmentRequest, ApproveCsrRequest, AuthServiceClient, CsrStatus, GetCsrStatusRequest,
    ListPendingCsrsRequest, ListUsersRequest, RevokeCertRequest, SubmitCsrRequest,
    connect_tls_insecure,
};
use crate::config::ClientConfig;
use crate::ui;

#[derive(Subcommand)]
pub enum AuthAction {
    Requests,
    Approve {
        fingerprint: String,
        #[arg(long, default_value = "vm:read")]
        permissions: String,
    },
    Revoke {
        fingerprint: String,
    },
    List,
}

/// Handles authentication commands.
pub async fn handle(channel: Channel, action: AuthAction) -> Result<()> {
    match action {
        AuthAction::Requests => requests(channel).await,
        AuthAction::Approve {
            fingerprint,
            permissions,
        } => approve(channel, &fingerprint, &permissions).await,
        AuthAction::Revoke { fingerprint } => revoke(channel, &fingerprint).await,
        AuthAction::List => list(channel).await,
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
        let display_fp = if csr.fingerprint.len() >= 16 {
            &csr.fingerprint[..16]
        } else {
            &csr.fingerprint
        };
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
        .map(|s| s.trim().to_string())
        .collect();

    let pending_response = auth_client
        .list_pending_csrs(ListPendingCsrsRequest {})
        .await
        .context("Failed to list pending CSRs")?;

    let csrs = pending_response.into_inner().csrs;
    let matching: Vec<_> = csrs
        .iter()
        .filter(|c| c.fingerprint.starts_with(fingerprint))
        .collect();

    let full_fingerprint = match matching.len() {
        0 => {
            println!(
                "{}: No pending CSR matches '{}'",
                ui::style::error("Error"),
                fingerprint
            );
            return Ok(());
        }
        1 => matching[0].fingerprint.clone(),
        _ => {
            println!(
                "{}: Multiple CSRs match '{}'. Please be more specific:",
                ui::style::error("Error"),
                fingerprint
            );
            for csr in matching {
                println!("  {}", csr.fingerprint);
            }
            return Ok(());
        }
    };

    let short_fp = full_fingerprint[..16].to_string();
    println!("Approving CSR: {}", ui::style::highlight(&short_fp));
    println!("Permissions: {:?}", perms);

    let _ = auth_client
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
        .filter(|u| u.fingerprint.starts_with(fingerprint))
        .collect();

    let full_fingerprint = match matching.len() {
        0 => {
            println!(
                "{}: No user matches fingerprint '{}'",
                ui::style::error("Error"),
                fingerprint
            );
            return Ok(());
        }
        1 => matching[0].fingerprint.clone(),
        _ => {
            println!(
                "{}: Multiple users match '{}'. Please be more specific:",
                ui::style::error("Error"),
                fingerprint
            );
            for user in matching {
                println!("  {}", user.fingerprint);
            }
            return Ok(());
        }
    };

    let short_fp = full_fingerprint[..16].to_string();
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
            let display_fp = if user.fingerprint.len() >= 16 {
                &user.fingerprint[..16]
            } else {
                &user.fingerprint
            };
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
            let display_fp = if fp.len() >= 16 { &fp[..16] } else { fp };
            println!("  {}", ui::style::muted(display_fp));
        }
    }

    Ok(())
}

/// Enrolls with a server by generating a CSR and polling for approval.
pub async fn enroll(endpoint: &str) -> Result<()> {
    let mut config = ClientConfig::load()?;

    if config.has_credentials_for_endpoint(endpoint) {
        println!("Already authenticated to {}.", endpoint);
        println!(
            "Use '{}' to re-enroll.",
            ui::style::accent("muakctl context remove <name>")
        );
        return Ok(());
    }

    let (fingerprint, key_pem) = if let Some(pending) = config.get_pending(endpoint) {
        println!(
            "Resuming pending enrollment for {}",
            ui::style::accent(endpoint)
        );
        let key = Base64::decode_vec(&pending.key).context("Failed to decode pending key")?;
        let key_pem = String::from_utf8(key).context("Invalid key encoding")?;
        (pending.fingerprint.clone(), key_pem)
    } else {
        println!("Generating key pair...");
        let (key_pem, csr_pem) = pki::generate_csr("muak-client")?;
        let fingerprint = pki::compute_csr_fingerprint(&csr_pem)?;

        config.start_enrollment(endpoint, &key_pem, &fingerprint);
        config.save()?;

        println!("Submitting certificate request...");
        let channel = connect_tls_insecure(endpoint, 30).await?;
        let mut client = AuthServiceClient::new(channel);

        client
            .submit_csr(SubmitCsrRequest { csr_pem })
            .await
            .context("Failed to submit CSR")?;

        (fingerprint, key_pem)
    };

    let display_fp = if fingerprint.len() >= 16 {
        &fingerprint[..16]
    } else {
        &fingerprint
    };
    println!("\nFingerprint: {}", ui::style::highlight(display_fp));
    println!(
        "Ask an admin to run: {} {}...",
        ui::style::positive("muakctl auth approve"),
        display_fp
    );

    let spinner = ui::Spinner::start("Waiting for admin approval... (Ctrl+C to resume later)");

    let channel = connect_tls_insecure(endpoint, 30).await?;
    let mut client = AuthServiceClient::new(channel);

    loop {
        let response = client
            .get_csr_status(GetCsrStatusRequest {
                fingerprint: fingerprint.clone(),
            })
            .await
            .context("Failed to check CSR status")?;

        let status = response.into_inner();
        match status.status() {
            CsrStatus::Pending => {
                // Spinner is already animating.
            }
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

                let _ = client
                    .ack_enrollment(AckEnrollmentRequest {
                        fingerprint: fingerprint.clone(),
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

        tokio::time::sleep(Duration::from_secs(3)).await;
    }
}
