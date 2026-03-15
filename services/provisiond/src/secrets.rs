//! LUKS key protection: TPM2 sealing, token management, and fallback embedding.

use anyhow::{Context, Result};

use crate::uki::Uki;

/// Result of sealing a LUKS key against a UKI.
pub enum SealResult {
    Sealed(luks2::Tpm2Token),
    Embedded,
}

/// Seals a LUKS key to TPM2 or embeds it in the UKI as a fallback.
pub fn seal_luks_key(key: &[u8], uki: &mut Uki) -> Result<SealResult> {
    if tpm2::is_available() {
        let token = seal_to_token(key, uki).context("Failed to seal LUKS key to TPM2")?;
        return Ok(SealResult::Sealed(token));
    }

    uki.luks_key = Some(key.to_vec());
    Ok(SealResult::Embedded)
}

/// Unseals the LUKS key from a TPM2 token stored in the LUKS2 header.
pub fn unseal_luks_key(state_device: Option<&str>) -> Option<Vec<u8>> {
    let token = luks2::read_tpm2_token(state_device?).ok()?;
    let blob_bytes = <base64ct::Base64 as base64ct::Encoding>::decode_vec(&token.tpm2_blob).ok()?;
    let blob = tpm2::SealedBlob::deserialize(&blob_bytes).ok()?;

    match tpm2::unseal(&blob) {
        Ok(key) => {
            println!("LUKS key unsealed from TPM2 for re-seal");
            Some(key)
        }
        Err(e) => {
            eprintln!("TPM2 unseal failed during update: {}", e);
            None
        }
    }
}

/// Resolves the LUKS key for an update and protects it via TPM2 seal or UKI embedding.
pub fn resolve_luks_key(
    uki: &mut Uki,
    state_device: Option<&str>,
    data_device: Option<&str>,
) -> Result<()> {
    let key = if tpm2::is_available() {
        unseal_luks_key(state_device)
    } else {
        None
    };

    let Some(key) = key else {
        if let Some(key) = read_luks_key_from_cmdline() {
            uki.luks_key = Some(key);
        }
        return Ok(());
    };

    match seal_luks_key(&key, uki)? {
        SealResult::Sealed(token) => {
            let devices: Vec<&str> = [state_device, data_device].into_iter().flatten().collect();
            write_token_to_devices(&token, &devices)?;
            println!("LUKS key re-sealed to TPM2 with new PCR#11 values");
        }
        SealResult::Embedded => {}
    }

    Ok(())
}

/// Writes a LUKS2 TPM2 token to each of the given device paths.
pub fn write_token_to_devices(token: &luks2::Tpm2Token, devices: &[&str]) -> Result<()> {
    for dev in devices {
        luks2::write_tpm2_token(dev, token)
            .with_context(|| format!("Failed to write TPM2 token to {}", dev))?;
    }
    Ok(())
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

/// Seals a LUKS key to TPM2 PCR#11 predicted from the given UKI and returns a LUKS2 token.
fn seal_to_token(luks_key: &[u8], uki: &Uki) -> Result<luks2::Tpm2Token> {
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
