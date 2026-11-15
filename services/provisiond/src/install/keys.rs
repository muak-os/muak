//! Install key material generation and LUKS key sealing.

use anyhow::{Result, bail};
use luks2::key::Passphrase;
use tokio::sync::mpsc;
use uki::measure::MeasuredSection;

use super::{pki, send_progress, volumes};
use crate::ipc::proto::provision::InstallProgress;
use crate::secrets;

pub(super) struct PkiResult {
    pub(super) client_result: pki::InstallResult,
    pub(super) auth_config: config::auth::State,
    pub(super) server_pki: pki::Server,
}

pub(super) async fn generate_keys(
    admin_csr_pem: &str,
    progress: &mpsc::Sender<InstallProgress>,
) -> Result<(Passphrase, PkiResult)> {
    send_progress(progress, "Generating encryption keys").await;
    let luks_key = luks2::key::generate()?;

    send_progress(progress, "Generating PKI and signing CSR").await;
    let ca = pki::generate_ca()?;
    let server_pki = pki::generate_server_cert(&ca)?;
    let (client_result, auth_config) = pki::sign_admin_csr(admin_csr_pem, &ca)?;

    Ok((
        luks_key,
        PkiResult {
            client_result,
            auth_config,
            server_pki,
        },
    ))
}

pub(super) fn seal_luks_key(
    luks_key: &[u8],
    tpm_available: bool,
    partitions: &volumes::PartitionInfo,
    sections: &[MeasuredSection],
) -> Result<()> {
    if !tpm_available {
        println!("TPM2 unavailable, LUKS key written to ESP");
        return Ok(());
    }

    match secrets::seal_luks_key(luks_key, sections)? {
        secrets::SealResult::Sealed(token) => {
            secrets::write_token_to_devices(&token, &[&partitions.state, &partitions.ephemeral])?;
            println!("LUKS key sealed to TPM2 with PCR#11 values");
            Ok(())
        }
        secrets::SealResult::EspKey => bail!("TPM available but seal returned EspKey"),
    }
}
