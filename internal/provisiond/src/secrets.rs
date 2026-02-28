//! LUKS key protection: TPM2 sealing, token management, and fallback embedding.

use anyhow::{Context, Result};

use crate::uki::Uki;

/// Seals a LUKS key to TPM2 PCR#11 predicted from the given UKI and returns a LUKS2 token.
pub fn seal_to_token(luks_key: &[u8], uki: &Uki) -> Result<luks2::Tpm2Token> {
    let section_data = uki
        .read_section_data()
        .context("Failed to read UKI sections for PCR prediction")?;
    let sections_ref: Vec<(&str, &[u8])> = section_data
        .iter()
        .map(|(name, data)| (name.as_str(), data.as_slice()))
        .collect();
    let expected_pcr = tpm2::pcr::predict_pcr11(&sections_ref);
    let (blob, policy_digest) =
        tpm2::seal(luks_key, &expected_pcr).context("Failed to seal LUKS key to TPM2")?;
    Ok(luks2::Tpm2Token {
        r#type: "tpm2".to_string(),
        keyslots: vec!["0".to_string()],
        tpm2_pcrs: vec![11],
        tpm2_hash_alg: "sha256".to_string(),
        tpm2_blob: <base64ct::Base64 as base64ct::Encoding>::encode_string(&blob.serialize()),
        tpm2_policy_hash: <base64ct::Base64 as base64ct::Encoding>::encode_string(&policy_digest),
    })
}

/// Writes a LUKS2 TPM2 token to each of the given device paths.
pub fn write_token_to_devices(token: &luks2::Tpm2Token, devices: &[&str]) -> Result<()> {
    for dev in devices {
        luks2::write_tpm2_token(dev, token)
            .with_context(|| format!("Failed to write TPM2 token to {}", dev))?;
    }
    Ok(())
}

/// Unseals the LUKS key from a TPM2 token stored in the LUKS2 header.
pub fn unseal_luks_key(state_device: Option<&str>) -> Option<Vec<u8>> {
    let token = luks2::read_tpm2_token(state_device?).ok()?;
    let blob_bytes = <base64ct::Base64 as base64ct::Encoding>::decode_vec(&token.tpm2_blob).ok()?;
    let blob = tpm2::SealedBlob::deserialize(&blob_bytes).ok()?;

    match tpm2::unseal(&blob) {
        Ok(key) => {
            kmsg::info!("LUKS key unsealed from TPM2 for re-seal");
            Some(key)
        }
        Err(e) => {
            kmsg::warn!("TPM2 unseal failed during update: {}", e);
            None
        }
    }
}

/// Returns the LUKS key to embed in the UKI, or `None` if TPM2 re-sealing was performed.
pub fn resolve_luks_key(
    uki: &mut Uki,
    state_device: Option<&str>,
    data_device: Option<&str>,
) -> Result<Option<Vec<u8>>> {
    if tpm2::is_available()
        && let Some(key) = unseal_luks_key(state_device)
    {
        let token = seal_to_token(&key, uki).context("Failed to re-seal LUKS key to TPM2")?;

        let devices: Vec<&str> = [state_device, data_device].into_iter().flatten().collect();
        write_token_to_devices(&token, &devices)?;

        kmsg::info!("LUKS key re-sealed to TPM2 with new PCR#11 values");
        return Ok(None);
    }

    Ok(read_luks_key_from_cmdline())
}

/// Reads the LUKS key from the current `/proc/cmdline`.
pub fn read_luks_key_from_cmdline() -> Option<Vec<u8>> {
    let cmdline = std::fs::read_to_string("/proc/cmdline").ok()?;
    let encoded = cmdline
        .split_whitespace()
        .find(|t| t.starts_with("luks.key="))?
        .strip_prefix("luks.key=")?;
    <base64ct::Base64Unpadded as base64ct::Encoding>::decode_vec(encoded).ok()
}
