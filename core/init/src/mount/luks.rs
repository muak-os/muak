//! LUKS key resolution.

use anyhow::{Context as _, Result};
use zeroize::Zeroizing;

/// Resolve the LUKS key for the given device, first trying TPM2 unseal and falling back to cmdline parsing.
pub(super) fn resolve_key(device: &str) -> Result<Option<Zeroizing<Vec<u8>>>> {
    match try_tpm2_unseal(device) {
        Ok(Some(key)) => Ok(Some(key)),
        Ok(None) => Ok(parse_luks_key()),
        Err(error) => Err(anyhow::anyhow!("TPM2 unseal failed: {error}")),
    }
}

fn try_tpm2_unseal(device: &str) -> Result<Option<Zeroizing<Vec<u8>>>> {
    if !tpm2::is_available() {
        return Ok(None);
    }

    let Ok(token) = luks2::read_tpm2_token(device) else {
        return Ok(None);
    };

    let blob_bytes = <base64ct::Base64 as base64ct::Encoding>::decode_vec(&token.tpm2_blob)
        .context("Failed to decode TPM2 blob from LUKS token")?;
    let blob =
        tpm2::SealedBlob::deserialize(&blob_bytes).context("Failed to deserialize TPM2 blob")?;

    match tpm2::unseal(&blob) {
        Ok(key) => {
            kmsg::info!("LUKS key unsealed from TPM2");
            Ok(Some(key))
        }
        Err(error) => {
            kmsg::error!("TPM2 unseal failed: {}", error);
            Err(anyhow::anyhow!("TPM2 unseal failed: {error}"))
        }
    }
}

fn parse_luks_key() -> Option<Zeroizing<Vec<u8>>> {
    let cmdline = std::fs::read_to_string("/proc/cmdline").ok()?;

    parse_luks_key_from_cmdline(&cmdline)
}

fn parse_luks_key_from_cmdline(cmdline: &str) -> Option<Zeroizing<Vec<u8>>> {
    let token = cmdline
        .split_whitespace()
        .find(|part| part.starts_with("luks.key="))?;

    let encoded = token.strip_prefix("luks.key=")?;

    <base64ct::Base64Unpadded as base64ct::Encoding>::decode_vec(encoded)
        .ok()
        .map(Zeroizing::new)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_luks_key_from_cmdline_decodes_valid_key() {
        // ARRANGE
        let key = b"secret-key-data";
        let encoded = <base64ct::Base64Unpadded as base64ct::Encoding>::encode_string(key);
        let cmdline = format!("quiet luks.key={encoded} splash");

        // ACT
        let result = parse_luks_key_from_cmdline(&cmdline);

        // ASSERT
        assert_eq!(
            result.as_ref().map(|value| value.as_slice()),
            Some(key.as_ref())
        );
    }

    #[test]
    fn parse_luks_key_from_cmdline_returns_none_when_absent() {
        // ARRANGE
        let cmdline = "quiet splash root=/dev/sda";

        // ACT & ASSERT
        assert!(parse_luks_key_from_cmdline(cmdline).is_none());
    }

    #[test]
    fn parse_luks_key_from_cmdline_returns_none_on_invalid_base64() {
        // ARRANGE
        let cmdline = "luks.key=!!!not-base64!!!";

        // ACT & ASSERT
        assert!(parse_luks_key_from_cmdline(cmdline).is_none());
    }

    #[test]
    fn parse_luks_key_from_cmdline_handles_empty_value() {
        // ARRANGE
        let cmdline = "luks.key=";

        // ACT & ASSERT
        assert_eq!(
            parse_luks_key_from_cmdline(cmdline)
                .as_ref()
                .map(|value| value.as_slice()),
            Some([].as_ref())
        );
    }

    #[test]
    fn parse_luks_key_from_cmdline_picks_first_matching_token() {
        // ARRANGE
        let key1 = b"first";
        let key2 = b"second";
        let enc1 = <base64ct::Base64Unpadded as base64ct::Encoding>::encode_string(key1);
        let enc2 = <base64ct::Base64Unpadded as base64ct::Encoding>::encode_string(key2);
        let cmdline = format!("luks.key={enc1} luks.key={enc2}");

        // ACT
        let result = parse_luks_key_from_cmdline(&cmdline);

        // ASSERT
        assert_eq!(
            result.as_ref().map(|value| value.as_slice()),
            Some(key1.as_ref())
        );
    }

    #[test]
    fn parse_luks_key_from_cmdline_rejects_garbled_suffix() {
        // ARRANGE
        let key = b"secret-key-data";
        let encoded = <base64ct::Base64Unpadded as base64ct::Encoding>::encode_string(key);
        let cmdline = format!("quiet luks.key={encoded}garbled splash");

        // ACT & ASSERT
        assert!(parse_luks_key_from_cmdline(&cmdline).is_none());
    }
}
