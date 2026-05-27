//! AuthService implementation for mTLS certificate enrollment and management.

use std::path::{Path, PathBuf};

use anyhow::Result;
use tonic::{Request, Response, Status};
use x509_cert::Certificate;
use x509_cert::der::{DecodePem, EncodePem, pem::LineEnding};

use super::proto::auth::auth_service_server::{AuthService, AuthServiceServer};
use super::proto::auth::get_csr_status_response::Status as CsrStatus;
use super::proto::auth::*;
use crate::constants::SECRETS_DIR;

fn pending_dir() -> PathBuf {
    Path::new(SECRETS_DIR).join("pending")
}

fn staging_dir() -> PathBuf {
    Path::new(SECRETS_DIR).join("staging")
}

fn ca_cert_path() -> PathBuf {
    Path::new(SECRETS_DIR).join("ca.crt")
}

fn ca_key_path() -> PathBuf {
    Path::new(SECRETS_DIR).join("ca.key")
}

pub fn service() -> AuthServiceServer<AuthServiceImpl> {
    AuthServiceServer::new(AuthServiceImpl)
}

pub struct AuthServiceImpl;

#[tonic::async_trait]
impl AuthService for AuthServiceImpl {
    async fn submit_csr(
        &self,
        request: Request<SubmitCsrRequest>,
    ) -> Result<Response<SubmitCsrResponse>, Status> {
        let req = request.into_inner();

        let fingerprint = pki::csr::compute_fingerprint(&req.csr_pem)
            .map_err(|e| Status::invalid_argument(format!("Invalid CSR: {}", e)))?;

        if is_user_authorized(&fingerprint) {
            return Err(Status::already_exists(
                "A certificate with this fingerprint is already authorized",
            ));
        }

        let pending_path = pending_csr_path(&fingerprint);
        if pending_path.exists() {
            println!("CSR already pending: {}", &fingerprint[..16]);
            return Ok(Response::new(SubmitCsrResponse { fingerprint }));
        }

        store_pending_csr(&fingerprint, &req.csr_pem)
            .await
            .map_err(|e| Status::internal(format!("Failed to store CSR: {}", e)))?;

        println!("CSR submitted: {}", &fingerprint[..16]);

        Ok(Response::new(SubmitCsrResponse { fingerprint }))
    }

    async fn get_csr_status(
        &self,
        request: Request<GetCsrStatusRequest>,
    ) -> Result<Response<GetCsrStatusResponse>, Status> {
        let fingerprint = request.into_inner().fingerprint;

        if let Some(auth) = config::try_auth()
            && auth.revoked.contains(&fingerprint)
        {
            return Ok(Response::new(GetCsrStatusResponse {
                status: CsrStatus::Rejected.into(),
                cert_pem: String::new(),
                ca_pem: String::new(),
                server_name: String::new(),
            }));
        }

        if let Ok((ca_pem, cert_pem)) = load_staging_cert(&fingerprint) {
            let server_name = config::try_config()
                .map(|c| c.host.name.clone())
                .unwrap_or_default();

            return Ok(Response::new(GetCsrStatusResponse {
                status: CsrStatus::Approved.into(),
                cert_pem,
                ca_pem,
                server_name,
            }));
        }

        let pending_path = pending_csr_path(&fingerprint);
        if pending_path.exists() {
            return Ok(Response::new(GetCsrStatusResponse {
                status: CsrStatus::Pending.into(),
                cert_pem: String::new(),
                ca_pem: String::new(),
                server_name: String::new(),
            }));
        }

        Ok(Response::new(GetCsrStatusResponse {
            status: CsrStatus::NotFound.into(),
            cert_pem: String::new(),
            ca_pem: String::new(),
            server_name: String::new(),
        }))
    }

    async fn list_pending_csrs(
        &self,
        _request: Request<ListPendingCsrsRequest>,
    ) -> Result<Response<ListPendingCsrsResponse>, Status> {
        let csrs = list_pending_csrs()
            .await
            .map_err(|e| Status::internal(format!("Failed to list pending CSRs: {}", e)))?;

        Ok(Response::new(ListPendingCsrsResponse { csrs }))
    }

    async fn approve_csr(
        &self,
        request: Request<ApproveCsrRequest>,
    ) -> Result<Response<ApproveCsrResponse>, Status> {
        let req = request.into_inner();
        let fingerprint = req.fingerprint;
        let permissions: Vec<String> = req.permissions;

        let pending_path = pending_csr_path(&fingerprint);
        if !pending_path.exists() {
            return Err(Status::not_found("CSR not found"));
        }

        let csr_pem = tokio::fs::read_to_string(&pending_path)
            .await
            .map_err(|e| Status::internal(format!("Failed to read CSR: {}", e)))?;

        let (cert, cert_fingerprint) =
            tokio::task::spawn_blocking(move || sign_pending_csr(&csr_pem))
                .await
                .map_err(|e| Status::internal(format!("Task failed: {}", e)))?
                .map_err(|e| Status::internal(format!("Failed to sign CSR: {}", e)))?;

        let cert_pem = cert
            .to_pem(LineEnding::LF)
            .map_err(|e| Status::internal(format!("Failed to encode certificate: {}", e)))?;

        store_staging_cert(&fingerprint, &cert_pem)
            .await
            .map_err(|e| Status::internal(format!("Failed to store certificate: {}", e)))?;

        let parsed_permissions: Vec<config::Permission> = {
            let mut perms = Vec::new();
            for pattern in &permissions {
                match config::Permission::expand_pattern(pattern) {
                    Ok(expanded) => perms.extend(expanded),
                    Err(e) => return Err(Status::invalid_argument(e)),
                }
            }
            perms
        };

        add_user_to_auth(&cert_fingerprint, parsed_permissions)
            .await
            .map_err(|e| Status::internal(format!("Failed to update auth config: {}", e)))?;

        let _ = tokio::fs::remove_file(&pending_path).await;

        println!(
            "CSR approved: {} -> {}",
            &fingerprint[..16],
            &cert_fingerprint[..16]
        );

        Ok(Response::new(ApproveCsrResponse { cert_pem }))
    }

    async fn revoke_cert(
        &self,
        request: Request<RevokeCertRequest>,
    ) -> Result<Response<RevokeCertResponse>, Status> {
        let fingerprint = request.into_inner().fingerprint;

        revoke_user(&fingerprint)
            .await
            .map_err(|e| Status::internal(format!("Failed to revoke certificate: {}", e)))?;

        println!("Certificate revoked: {}", &fingerprint[..16]);

        Ok(Response::new(RevokeCertResponse {}))
    }

    async fn list_users(
        &self,
        _request: Request<ListUsersRequest>,
    ) -> Result<Response<ListUsersResponse>, Status> {
        let auth = config::try_auth().ok_or_else(|| {
            Status::failed_precondition("System not installed - auth config not available")
        })?;

        let users: Vec<AuthorizedUser> = auth
            .users
            .iter()
            .map(|u| AuthorizedUser {
                fingerprint: u.fingerprint.clone(),
                permissions: config::permission::collapse(&u.permissions),
            })
            .collect();

        let revoked_fingerprints = auth.revoked.clone();

        Ok(Response::new(ListUsersResponse {
            users,
            revoked_fingerprints,
        }))
    }

    async fn ack_enrollment(
        &self,
        request: Request<AckEnrollmentRequest>,
    ) -> Result<Response<AckEnrollmentResponse>, Status> {
        let fingerprint = request.into_inner().fingerprint;

        let cert_path = staging_cert_path(&fingerprint);
        if cert_path.exists() {
            tokio::fs::remove_file(&cert_path)
                .await
                .map_err(|e| Status::internal(format!("Failed to cleanup staging cert: {}", e)))?;
        }

        println!("Enrollment acknowledged: {}", &fingerprint[..16]);

        Ok(Response::new(AckEnrollmentResponse {}))
    }
}

/// Returns the path for a pending CSR.
fn pending_csr_path(fingerprint: &str) -> PathBuf {
    pending_dir().join(format!("{}.pem", fingerprint))
}

/// Stores a pending CSR to disk.
async fn store_pending_csr(fingerprint: &str, csr_pem: &str) -> anyhow::Result<()> {
    tokio::fs::create_dir_all(pending_dir()).await?;
    let path = pending_csr_path(fingerprint);
    tokio::fs::write(path, csr_pem).await?;
    Ok(())
}

/// Lists all pending CSRs.
async fn list_pending_csrs() -> Result<Vec<PendingCsr>> {
    let dir = pending_dir();
    if !tokio::fs::try_exists(&dir).await? {
        return Ok(Vec::new());
    }

    let mut csrs = Vec::new();
    let mut entries = tokio::fs::read_dir(dir).await?;

    while let Some(entry) = entries.next_entry().await? {
        let path = entry.path();

        if path.extension().is_some_and(|e| e == "pem")
            && let Some(stem) = path.file_stem().and_then(|s| s.to_str())
        {
            let metadata = tokio::fs::metadata(&path).await?;
            let submitted_at = metadata
                .created()
                .or_else(|_| metadata.modified())
                .map(|t| {
                    t.duration_since(std::time::UNIX_EPOCH)
                        .map(|d| d.as_secs().to_string())
                        .unwrap_or_default()
                })
                .unwrap_or_default();

            csrs.push(PendingCsr {
                fingerprint: stem.to_string(),
                submitted_at,
            });
        }
    }

    Ok(csrs)
}

/// Signs pending CSR with the CA.
fn sign_pending_csr(csr_pem: &str) -> Result<(Certificate, String)> {
    let ca_key_pem = std::fs::read_to_string(ca_key_path())
        .map_err(|e| anyhow::anyhow!("CA key not found: {}", e))?;
    let ca_cert_pem = std::fs::read_to_string(ca_cert_path())
        .map_err(|e| anyhow::anyhow!("CA cert not found: {}", e))?;
    let ca_cert = Certificate::from_pem(&ca_cert_pem)?;

    let (cert, fingerprint) = pki::csr::sign(csr_pem, &ca_key_pem, &ca_cert)?;
    Ok((cert, fingerprint))
}

/// Checks if a fingerprint is already authorized.
fn is_user_authorized(fingerprint: &str) -> bool {
    config::try_auth()
        .map(|a| a.users.iter().any(|u| u.fingerprint == fingerprint))
        .unwrap_or(false)
}

/// Returns the path for a staging certificate.
fn staging_cert_path(fingerprint: &str) -> PathBuf {
    staging_dir().join(format!("{}.crt", fingerprint))
}

/// Stores a certificate in staging for client pickup.
async fn store_staging_cert(fingerprint: &str, cert_pem: &str) -> Result<()> {
    tokio::fs::create_dir_all(staging_dir()).await?;
    let path = staging_cert_path(fingerprint);
    tokio::fs::write(path, cert_pem).await?;
    Ok(())
}

/// Loads a staging certificate and CA cert for a fingerprint.
fn load_staging_cert(fingerprint: &str) -> Result<(String, String)> {
    let ca_pem = std::fs::read_to_string(ca_cert_path())?;
    let cert_pem = std::fs::read_to_string(staging_cert_path(fingerprint))?;
    Ok((ca_pem, cert_pem))
}

/// Adds a user to the auth config and writes it to disk.
async fn add_user_to_auth(fingerprint: &str, permissions: Vec<config::Permission>) -> Result<()> {
    let mut auth = config::try_auth().map(|a| (*a).clone()).unwrap_or_default();

    auth.users.push(config::AuthUser {
        fingerprint: fingerprint.to_string(),
        permissions,
    });

    let auth_str = config::serialize_auth(&auth)?;
    tokio::fs::write(config::AUTH_PATH, auth_str).await?;

    Ok(())
}

/// Revokes a user by adding their fingerprint to the revoked list.
async fn revoke_user(fingerprint: &str) -> Result<()> {
    let mut auth = config::try_auth()
        .map(|a| config::AuthConfig::clone(&a))
        .unwrap_or_default();

    auth.users.retain(|u| u.fingerprint != fingerprint);

    if !auth.revoked.contains(&fingerprint.to_string()) {
        auth.revoked.push(fingerprint.to_string());
    }

    let auth_str = config::serialize_auth(&auth)?;
    tokio::fs::write(config::AUTH_PATH, auth_str).await?;

    Ok(())
}
