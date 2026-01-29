//! Authentication commands for certificate management (admin only).

use anyhow::{Context, Result};
use owo_colors::OwoColorize;

use crate::AuthAction;
use crate::client::{
    self, ApproveCsrRequest, AuthServiceClient, ListPendingCsrsRequest, ListUsersRequest,
    RevokeCertRequest,
};

/// Handles authentication commands.
pub async fn handle(server: &str, action: AuthAction) -> Result<()> {
    match action {
        AuthAction::Requests => requests(server).await,
        AuthAction::Approve {
            fingerprint,
            permissions,
        } => approve(server, &fingerprint, &permissions).await,
        AuthAction::Revoke { fingerprint } => revoke(server, &fingerprint).await,
        AuthAction::List => list(server).await,
    }
}

/// Lists pending authentication requests (admin only).
async fn requests(server: &str) -> Result<()> {
    let channel = client::connect(server, 30).await?;
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

    println!(
        "{} pending authentication request(s):\n",
        csrs.len().to_string().cyan()
    );

    for csr in csrs {
        let display_fp = if csr.fingerprint.len() >= 16 {
            &csr.fingerprint[..16]
        } else {
            &csr.fingerprint
        };
        println!(
            "  {} (submitted: {})",
            display_fp.yellow(),
            csr.submitted_at
        );
    }

    println!(
        "\nTo approve: {} <fingerprint> --permissions admin",
        "muakctl auth approve".green()
    );

    Ok(())
}

/// Approves a pending authentication request (admin only).
async fn approve(server: &str, fingerprint: &str, permissions: &str) -> Result<()> {
    let channel = client::connect(server, 30).await?;
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
                "Error".red(),
                fingerprint
            );
            return Ok(());
        }
        1 => matching[0].fingerprint.clone(),
        _ => {
            println!(
                "{}: Multiple CSRs match '{}'. Please be more specific:",
                "Error".red(),
                fingerprint
            );
            for csr in matching {
                println!("  {}", csr.fingerprint);
            }
            return Ok(());
        }
    };

    println!(
        "Approving CSR: {}",
        full_fingerprint[..16].to_string().yellow()
    );
    println!("Permissions: {:?}", perms);

    let response = auth_client
        .approve_csr(ApproveCsrRequest {
            fingerprint: full_fingerprint.clone(),
            permissions: perms,
        })
        .await
        .context("Failed to approve CSR")?;

    let cert_pem = response.into_inner().cert_pem;

    println!("\n{} Certificate issued!", "Success:".green());
    println!(
        "Certificate (first 100 chars): {}...",
        &cert_pem[..cert_pem.len().min(100)]
    );

    Ok(())
}

/// Revokes a certificate (admin only).
async fn revoke(server: &str, fingerprint: &str) -> Result<()> {
    let channel = client::connect(server, 30).await?;
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
                "Error".red(),
                fingerprint
            );
            return Ok(());
        }
        1 => matching[0].fingerprint.clone(),
        _ => {
            println!(
                "{}: Multiple users match '{}'. Please be more specific:",
                "Error".red(),
                fingerprint
            );
            for user in matching {
                println!("  {}", user.fingerprint);
            }
            return Ok(());
        }
    };

    println!(
        "Revoking certificate: {}",
        full_fingerprint[..16].to_string().yellow()
    );

    auth_client
        .revoke_cert(RevokeCertRequest {
            fingerprint: full_fingerprint.clone(),
        })
        .await
        .context("Failed to revoke certificate")?;

    println!("{} Certificate revoked.", "Success:".green());

    Ok(())
}

/// Lists all authorized users (admin only).
async fn list(server: &str) -> Result<()> {
    let channel = client::connect(server, 30).await?;
    let mut auth_client = AuthServiceClient::new(channel);

    let response = auth_client
        .list_users(ListUsersRequest {})
        .await
        .context("Failed to list users")?;

    let inner = response.into_inner();

    if inner.users.is_empty() {
        println!("No authorized users.");
    } else {
        println!(
            "{} authorized user(s):\n",
            inner.users.len().to_string().cyan()
        );

        for user in &inner.users {
            let display_fp = if user.fingerprint.len() >= 16 {
                &user.fingerprint[..16]
            } else {
                &user.fingerprint
            };
            let perms = user.permissions.join(", ");
            println!("  {} [{}]", display_fp.green(), perms.dimmed());
        }
    }

    if !inner.revoked_fingerprints.is_empty() {
        println!(
            "\n{} revoked certificate(s):",
            inner.revoked_fingerprints.len().to_string().red()
        );
        for fp in &inner.revoked_fingerprints {
            let display_fp = if fp.len() >= 16 { &fp[..16] } else { fp };
            println!("  {}", display_fp.dimmed());
        }
    }

    Ok(())
}
