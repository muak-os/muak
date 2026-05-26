//! Interact with TPM2 via `/dev/tpmrm0`.

mod auth;
mod blob;
mod buffer;
mod commands;
mod device;
mod error;
mod handles;
mod operations;
pub mod pcr;
mod response;

pub type Result<T> = error::Result<T>;
pub type SealedBlob = blob::SealedBlob;
pub type SealResult = operations::SealResult;
pub type Tpm2Error = error::Tpm2Error;

/// Returns true if the TPM2 resource manager device exists.
#[must_use]
pub fn is_available() -> bool {
    device::is_available(None)
}

/// Seals data to PCR#11 with the given expected PCR value.
///
/// # Errors
///
/// Returns an error if TPM access or command execution fails.
pub fn seal(data: &[u8], expected_pcr: &pcr::Digest) -> Result<SealResult> {
    operations::seal(data, expected_pcr)
}

/// Unseals data using current PCR#11 values.
///
/// # Errors
///
/// Returns an error if TPM access, object loading, policy setup, or unsealing fails.
pub fn unseal(blob: &SealedBlob) -> Result<zeroize::Zeroizing<Vec<u8>>> {
    operations::unseal(blob)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn public_api_stays_connected() {
        // ARRANGE
        let sections = [(".linux", &[1_u8, 2][..])];
        let pcr = [0x42_u8; 32];
        let blob = match SealedBlob::try_new(Vec::new(), Vec::new()) {
            Ok(blob) => blob,
            Err(_) => panic!("empty sealed blob should be valid"),
        };

        // ACT
        let predicted = pcr::predict_pcr11(&sections);
        let available = is_available();
        let sealed = seal(&[], &pcr);
        let unsealed = unseal(&blob);

        // ASSERT
        assert_eq!(
            predicted.len(),
            32,
            "PCR helper should return SHA-256 length"
        );
        assert_eq!(
            available,
            device::is_available(None),
            "availability wrapper should match"
        );
        assert!(sealed.is_err(), "seal should fail without a TPM in tests");
        assert!(
            unsealed.is_err(),
            "unseal should fail without a TPM in tests"
        );
    }
}
