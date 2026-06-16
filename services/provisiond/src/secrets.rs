//! LUKS key protection: TPM2 sealing, token management, and fallback to ESP file.

use anyhow::{Context, Result};
use imager::build::Section;
use luks2::Tpm2Token;
use zeroize::Zeroizing;

/// Result of sealing a LUKS key against a UKI.
pub enum SealResult {
    Sealed(Tpm2Token),
    EspKey,
}

/// Seals a LUKS key to TPM2 or signals writing it to the ESP as a fallback.
pub fn seal_luks_key(key: &[u8], uki_bytes: &[u8], sections: &[Section]) -> Result<SealResult> {
    if tpm2::is_available() {
        let token =
            seal_to_token(key, uki_bytes, sections).context("Failed to seal LUKS key to TPM2")?;
        return Ok(SealResult::Sealed(token));
    }

    Ok(SealResult::EspKey)
}

/// Unseals the LUKS key from a TPM2 token stored in the LUKS2 header.
pub fn unseal_luks_key(state_device: Option<&str>) -> Option<Zeroizing<Vec<u8>>> {
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

/// Resolves the LUKS key for an update: unseal from TPM2 or read from cmdline.
pub fn resolve_luks_key(state_device: Option<&str>) -> Option<Zeroizing<Vec<u8>>> {
    if tpm2::is_available() {
        if let Some(key) = unseal_luks_key(state_device) {
            return Some(key);
        }
    }

    read_luks_key_from_cmdline().map(Zeroizing::new)
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

/// Seals a LUKS key to TPM2 PCR#11 predicted from the UKI and returns a LUKS2 token.
fn seal_to_token(
    luks_key: &[u8],
    uki_bytes: &[u8],
    sections: &[Section],
) -> Result<luks2::Tpm2Token> {
    let sections: Vec<(&str, &[u8])> = sections
        .iter()
        .map(|s| (s.name, &uki_bytes[s.file_offset..s.file_offset + s.size]))
        .collect();
    let expected_pcr = tpm2::pcr::predict_pcr11(&sections);
    let sealed = tpm2::seal(luks_key, &expected_pcr).context("Failed to seal LUKS key to TPM2")?;

    Ok(Tpm2Token {
        r#type: "tpm2".to_string(),
        keyslots: vec!["0".to_string()],
        tpm2_pcrs: vec![11],
        tpm2_hash_alg: "sha256".to_string(),
        tpm2_blob: <base64ct::Base64 as base64ct::Encoding>::encode_string(
            &sealed.blob.serialize(),
        ),
        tpm2_policy_hash: <base64ct::Base64 as base64ct::Encoding>::encode_string(
            &sealed.policy_digest,
        ),
    })
}
