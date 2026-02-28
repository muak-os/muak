//! High-level TPM2 seal/unseal operations.

use zeroize::Zeroize;

use crate::commands;
use crate::device::Device;
use crate::errors::{Error, Result};
use crate::pcr;
use crate::types::{SHA256_DIGEST_SIZE, SRK_HANDLE};

/// Sealed blob format: [pub_size:u16][pub_data][priv_size:u16][priv_data].
pub struct SealedBlob {
    pub pub_data: Vec<u8>,
    pub priv_data: Vec<u8>,
}

impl SealedBlob {
    /// Serializes into wire format.
    pub fn serialize(&self) -> Vec<u8> {
        let mut buf = Vec::with_capacity(4 + self.pub_data.len() + self.priv_data.len());
        buf.extend_from_slice(&(self.pub_data.len() as u16).to_le_bytes());
        buf.extend_from_slice(&self.pub_data);
        buf.extend_from_slice(&(self.priv_data.len() as u16).to_le_bytes());
        buf.extend_from_slice(&self.priv_data);
        buf
    }

    /// Deserializes from wire format.
    pub fn deserialize(data: &[u8]) -> Result<Self> {
        if data.len() < 4 {
            return Err(Error::InvalidBlob);
        }

        let pub_size = u16::from_le_bytes([data[0], data[1]]) as usize;
        if data.len() < 2 + pub_size + 2 {
            return Err(Error::InvalidBlob);
        }

        let pub_data = data[2..2 + pub_size].to_vec();
        let priv_offset = 2 + pub_size;

        let priv_size = u16::from_le_bytes([data[priv_offset], data[priv_offset + 1]]) as usize;
        if data.len() < priv_offset + 2 + priv_size {
            return Err(Error::InvalidBlob);
        }

        let priv_data = data[priv_offset + 2..priv_offset + 2 + priv_size].to_vec();

        Ok(Self {
            pub_data,
            priv_data,
        })
    }
}

/// Ensures the SRK exists at the well-known persistent handle.
fn ensure_srk(dev: &mut Device) -> Result<()> {
    if commands::handle_exists(dev, SRK_HANDLE)? {
        return Ok(());
    }

    let (transient_handle, _pub) = commands::create_primary(dev)?;

    let result = commands::evict_control(
        dev,
        crate::types::TPM2_RH_OWNER,
        transient_handle,
        SRK_HANDLE,
    );
    let _ = commands::flush_context(dev, transient_handle);
    result?;
    Ok(())
}

/// Seals data to PCR#11 with the given expected PCR value.
pub fn seal(
    data: &[u8],
    expected_pcr: &[u8; SHA256_DIGEST_SIZE],
) -> Result<(SealedBlob, [u8; SHA256_DIGEST_SIZE])> {
    let mut dev = Device::open()?;
    ensure_srk(&mut dev)?;

    let policy_digest = pcr::compute_policy_digest(expected_pcr);

    let (pub_data, priv_data) = commands::create(&mut dev, SRK_HANDLE, &policy_digest, data)?;

    let blob = SealedBlob {
        pub_data,
        priv_data,
    };
    Ok((blob, policy_digest))
}

/// Unseals data using current PCR#11 values.
pub fn unseal(blob: &SealedBlob) -> Result<Vec<u8>> {
    let mut dev = Device::open()?;
    ensure_srk(&mut dev)?;

    let obj_handle = commands::load(&mut dev, SRK_HANDLE, &blob.pub_data, &blob.priv_data)?;

    let session = commands::start_auth_session(&mut dev)?;

    let result = (|| -> Result<Vec<u8>> {
        commands::policy_pcr(&mut dev, session, &[])?;
        commands::unseal(&mut dev, obj_handle, session)
    })();

    let _ = commands::flush_context(&mut dev, session);
    let _ = commands::flush_context(&mut dev, obj_handle);

    result
}

/// Seals data to predicted PCR#11 values computed from UKI sections.
pub fn seal_to_pcr11(
    data: &[u8],
    sections: &[(&str, &[u8])],
) -> Result<(Vec<u8>, [u8; SHA256_DIGEST_SIZE])> {
    let expected_pcr = pcr::predict_pcr11(sections);
    let (blob, policy_digest) = seal(data, &expected_pcr)?;
    Ok((blob.serialize(), policy_digest))
}

/// Unseals data from a serialized blob using current PCR#11 values.
pub fn unseal_from_blob(blob_data: &[u8]) -> Result<Vec<u8>> {
    let blob = SealedBlob::deserialize(blob_data)?;
    let mut result = unseal(&blob)?;
    let out = result.clone();
    result.zeroize();
    Ok(out)
}

/// Reads the current PCR#11 SHA-256 value from the TPM.
pub fn read_pcr11() -> Result<[u8; SHA256_DIGEST_SIZE]> {
    let mut dev = Device::open()?;
    commands::read_pcr(&mut dev, crate::types::PCR_INDEX)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sealed_blob_roundtrip() {
        let blob = SealedBlob {
            pub_data: vec![1, 2, 3, 4, 5],
            priv_data: vec![10, 20, 30],
        };

        let serialized = blob.serialize();
        let deserialized = SealedBlob::deserialize(&serialized).expect("should deserialize");

        assert_eq!(deserialized.pub_data, vec![1, 2, 3, 4, 5]);
        assert_eq!(deserialized.priv_data, vec![10, 20, 30]);
    }

    #[test]
    fn test_sealed_blob_empty() {
        let blob = SealedBlob {
            pub_data: vec![],
            priv_data: vec![],
        };

        let serialized = blob.serialize();
        let deserialized = SealedBlob::deserialize(&serialized).expect("should deserialize");

        assert!(deserialized.pub_data.is_empty());
        assert!(deserialized.priv_data.is_empty());
    }

    #[test]
    fn test_sealed_blob_invalid() {
        assert!(SealedBlob::deserialize(&[]).is_err());
        assert!(SealedBlob::deserialize(&[0, 0]).is_err());
        assert!(SealedBlob::deserialize(&[5, 0, 1]).is_err());
    }
}
