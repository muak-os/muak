//! PKI generation and CSR signing for installation.

use anyhow::{Context as _, Result};
use config::{AuthConfig, AuthUser, Permission};
use pki::cert;
use pki::csr;
use pki::key::Signer;
use pki::pem;

/// PKI materials returned to the client after install.
pub struct InstallResult {
    pub ca_pem: String,
    pub admin_cert_pem: String,
}

/// Server-side PKI materials written to the STATE partition.
pub struct Server {
    pub ca: String,
    pub ca_key: String,
    pub cert: String,
    pub key: String,
}

/// Intermediate CA materials shared across generation steps.
pub struct CaMaterials {
    pub signer: Signer,
    pub cert_pem: String,
    pub key_pem: String,
}

/// Generates the CA key and certificate.
pub fn generate_ca() -> Result<CaMaterials> {
    let (signer, cert) =
        cert::generate_ca("Muak CA").context("Failed to generate CA certificate")?;

    let cert_pem = pem::encode_cert(&cert).context("Failed to encode CA certificate")?;

    let key_pem = pem::encode_pkcs8(signer.pkcs8_der()).context("Failed to encode CA key")?;

    Ok(CaMaterials {
        signer,
        cert_pem,
        key_pem,
    })
}

/// Generates the server certificate signed by the given CA.
pub fn generate_server_cert(ca: &CaMaterials) -> Result<Server> {
    let ca_cert = pem::decode_cert(&ca.cert_pem).context("Failed to decode CA certificate")?;

    let (server_key, server_cert) = cert::generate_server("muak-server", &ca.signer, &ca_cert)
        .context("Failed to generate server certificate")?;

    let server_cert_pem =
        pem::encode_cert(&server_cert).context("Failed to encode server certificate")?;

    let server_key_pem =
        pem::encode_pkcs8(server_key.pkcs8_der()).context("Failed to encode server key")?;

    Ok(Server {
        ca: ca.cert_pem.clone(),
        ca_key: ca.key_pem.clone(),
        cert: server_cert_pem,
        key: server_key_pem,
    })
}

/// Signs the admin CSR with the given CA, returning client materials and initial auth config.
pub fn sign_admin_csr(csr_pem: &str, ca: &CaMaterials) -> Result<(InstallResult, AuthConfig)> {
    let ca_cert = pem::decode_cert(&ca.cert_pem).context("Failed to decode CA certificate")?;

    let (admin_cert, admin_fingerprint) =
        csr::sign(csr_pem, &ca.key_pem, &ca_cert).context("Failed to sign admin CSR")?;

    let admin_cert_pem =
        pem::encode_cert(&admin_cert).context("Failed to encode admin certificate")?;

    let auth_config = AuthConfig {
        users: vec![AuthUser {
            fingerprint: admin_fingerprint,
            permissions: vec![Permission::Admin],
        }],
        revoked: vec![],
    };

    Ok((
        InstallResult {
            ca_pem: ca.cert_pem.clone(),
            admin_cert_pem,
        },
        auth_config,
    ))
}
