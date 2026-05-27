//! PKI generation and CSR signing for installation.

use anyhow::{Context, Result};
use config::{AuthConfig, AuthUser, Permission};
use ring::rand::SecureRandom;
use x509_cert::Certificate;
use x509_cert::der::EncodePem;
use x509_cert::der::pem::LineEnding;

/// Size of the LUKS key in bytes.
const LUKS_KEY_SIZE: usize = 64;

/// PKI materials returned to the client after install.
pub struct InstallResult {
    pub ca_pem: String,
    pub admin_cert_pem: String,
}

/// Server-side PKI materials written to the STATE partition.
pub struct ServerPki {
    pub ca_pem: String,
    pub ca_key_pem: String,
    pub server_cert_pem: String,
    pub server_key_pem: String,
}

/// Intermediate CA materials shared across generation steps.
pub struct CaMaterials {
    pub signer: pki::key::Signer,
    pub cert: Certificate,
    pub pem: String,
    pub key_pem: String,
}

/// Generates the CA key and certificate.
pub fn generate_ca() -> Result<CaMaterials> {
    let (signer, cert) =
        pki::cert::generate_ca("Muak CA").context("Failed to generate CA certificate")?;

    let pem = cert
        .to_pem(LineEnding::LF)
        .context("Failed to encode CA certificate")?;

    let key_pem = pki::pem::encode_pkcs8(signer.pkcs8_der()).context("Failed to encode CA key")?;

    Ok(CaMaterials {
        signer,
        cert,
        pem,
        key_pem,
    })
}

/// Generates the server certificate signed by the given CA.
pub fn generate_server_cert(ca: &CaMaterials) -> Result<ServerPki> {
    let (server_key, server_cert) = pki::cert::generate_server("muak-server", &ca.signer, &ca.cert)
        .context("Failed to generate server certificate")?;

    let server_cert_pem = server_cert
        .to_pem(LineEnding::LF)
        .context("Failed to encode server certificate")?;

    let server_key_pem =
        pki::pem::encode_pkcs8(server_key.pkcs8_der()).context("Failed to encode server key")?;

    Ok(ServerPki {
        ca_pem: ca.pem.clone(),
        ca_key_pem: ca.key_pem.clone(),
        server_cert_pem,
        server_key_pem,
    })
}

/// Signs the admin CSR with the given CA, returning client materials and initial auth config.
pub fn sign_admin_csr(csr_pem: &str, ca: &CaMaterials) -> Result<(InstallResult, AuthConfig)> {
    let (admin_cert, admin_fingerprint) =
        pki::csr::sign(csr_pem, &ca.key_pem, &ca.cert).context("Failed to sign admin CSR")?;

    let admin_cert_pem = admin_cert
        .to_pem(LineEnding::LF)
        .context("Failed to encode admin certificate")?;

    let auth_config = AuthConfig {
        users: vec![AuthUser {
            fingerprint: admin_fingerprint,
            permissions: vec![Permission::Admin],
        }],
        revoked: vec![],
    };

    Ok((
        InstallResult {
            ca_pem: ca.pem.clone(),
            admin_cert_pem,
        },
        auth_config,
    ))
}

/// Generates a random LUKS key.
pub fn generate_luks_key() -> Result<Vec<u8>> {
    let rng = ring::rand::SystemRandom::new();
    let mut key = vec![0u8; LUKS_KEY_SIZE];
    rng.fill(&mut key)
        .map_err(|_| anyhow::anyhow!("Failed to generate random LUKS key"))?;
    Ok(key)
}
