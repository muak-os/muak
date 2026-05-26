//! Sealed blob storage format.

use crate::buffer::u16_len;
use crate::error::{Result, Tpm2Error};

/// Sealed blob format: `[pub_size:u16][pub_data][priv_size:u16][priv_data]`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SealedBlob {
    pub_data: Vec<u8>,
    priv_data: Vec<u8>,
}

impl SealedBlob {
    /// Creates a validated sealed blob.
    ///
    /// # Errors
    ///
    /// Returns an error if either blob part exceeds TPM2 sealed blob size limits.
    pub fn try_new(pub_data: Vec<u8>, priv_data: Vec<u8>) -> Result<Self> {
        u16_len(pub_data.len())?;
        u16_len(priv_data.len())?;
        Ok(Self {
            pub_data,
            priv_data,
        })
    }

    #[must_use]
    pub fn public(&self) -> &[u8] {
        &self.pub_data
    }

    #[must_use]
    pub fn private(&self) -> &[u8] {
        &self.priv_data
    }

    /// Serializes into wire format.
    #[must_use]
    pub fn serialize(&self) -> Vec<u8> {
        let capacity = 4_usize
            .checked_add(self.pub_data.len())
            .and_then(|value| value.checked_add(self.priv_data.len()))
            .unwrap_or(4);
        let mut buf = Vec::with_capacity(capacity);
        let pub_size = u16_len(self.pub_data.len()).unwrap_or(u16::MAX);
        buf.extend_from_slice(&pub_size.to_le_bytes());
        buf.extend_from_slice(&self.pub_data);
        let priv_size = u16_len(self.priv_data.len()).unwrap_or(u16::MAX);
        buf.extend_from_slice(&priv_size.to_le_bytes());
        buf.extend_from_slice(&self.priv_data);
        buf
    }

    /// Deserializes from wire format.
    ///
    /// # Errors
    ///
    /// Returns an error if the data does not contain a complete sealed blob.
    pub fn deserialize(data: &[u8]) -> Result<Self> {
        let (pub_size_bytes, rest) = data.split_at_checked(2).ok_or(Tpm2Error::InvalidBlob)?;
        let pub_size = usize::from(u16_from_le_slice(pub_size_bytes)?);
        let (pub_data, rest) = rest
            .split_at_checked(pub_size)
            .ok_or(Tpm2Error::InvalidBlob)?;
        let (priv_size_bytes, rest) = rest.split_at_checked(2).ok_or(Tpm2Error::InvalidBlob)?;
        let priv_size = usize::from(u16_from_le_slice(priv_size_bytes)?);
        let (priv_data, _trailing) = rest
            .split_at_checked(priv_size)
            .ok_or(Tpm2Error::InvalidBlob)?;

        Self::try_new(pub_data.to_vec(), priv_data.to_vec())
    }
}

fn u16_from_le_slice(bytes: &[u8]) -> Result<u16> {
    let mut array = [0_u8; 2];
    if bytes.len() != array.len() {
        return Err(Tpm2Error::InvalidBlob);
    }
    array.copy_from_slice(bytes);
    Ok(u16::from_le_bytes(array))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sealed_blob_roundtrip() {
        // ARRANGE
        let blob = SealedBlob::try_new(vec![1, 2, 3, 4, 5], vec![10, 20, 30]);

        // ACT
        let serialized = blob.as_ref().ok().map(SealedBlob::serialize);
        let deserialized = serialized
            .as_deref()
            .map(SealedBlob::deserialize)
            .unwrap_or_else(|| Err(Tpm2Error::InvalidBlob));

        // ASSERT
        assert!(blob.is_ok(), "blob should be valid");
        assert!(deserialized.is_ok(), "blob should deserialize");
        let deserialized = deserialized.unwrap_or_else(|_| panic!("blob should deserialize"));
        assert_eq!(
            deserialized.public(),
            &[1, 2, 3, 4, 5],
            "public data should match"
        );
        assert_eq!(
            deserialized.private(),
            &[10, 20, 30],
            "private data should match"
        );
    }

    #[test]
    fn sealed_blob_empty() {
        // ARRANGE
        let blob = SealedBlob::try_new(vec![], vec![]);

        // ACT
        let serialized = blob.as_ref().ok().map(SealedBlob::serialize);
        let deserialized = serialized
            .as_deref()
            .map(SealedBlob::deserialize)
            .unwrap_or_else(|| Err(Tpm2Error::InvalidBlob));

        // ASSERT
        assert!(blob.is_ok(), "empty blob should be valid");
        assert!(deserialized.is_ok(), "empty blob should deserialize");
        let deserialized = deserialized.unwrap_or_else(|_| panic!("blob should deserialize"));
        assert!(
            deserialized.public().is_empty(),
            "public data should be empty"
        );
        assert!(
            deserialized.private().is_empty(),
            "private data should be empty"
        );
    }

    #[test]
    fn sealed_blob_invalid() {
        // ACT & ASSERT
        assert!(
            SealedBlob::deserialize(&[]).is_err(),
            "empty blob should fail"
        );
        assert!(
            SealedBlob::deserialize(&[0, 0]).is_err(),
            "missing private size should fail"
        );
        assert!(
            SealedBlob::deserialize(&[5, 0, 1]).is_err(),
            "truncated public data should fail"
        );
    }

    #[test]
    fn sealed_blob_rejects_truncated_private_data() {
        // ARRANGE
        let data = [1, 0, 0xAA, 2, 0, 0xBB];

        // ACT
        let result = SealedBlob::deserialize(&data);

        // ASSERT
        assert!(result.is_err(), "truncated private data should fail");
    }

    #[test]
    fn sealed_blob_allows_trailing_data() {
        // ARRANGE
        let data = [1, 0, 0xAA, 1, 0, 0xBB, 0xCC];

        // ACT
        let result = SealedBlob::deserialize(&data);

        // ASSERT
        assert!(result.is_ok(), "trailing data should be ignored");
        let blob = result.unwrap_or_else(|_| panic!("blob should deserialize"));
        assert_eq!(blob.public(), &[0xAA], "public data should match");
        assert_eq!(blob.private(), &[0xBB], "private data should match");
    }

    #[test]
    fn try_new_rejects_oversized_sections() {
        // ARRANGE
        let oversized = vec![0_u8; usize::from(u16::MAX) + 1];

        // ACT
        let public_result = SealedBlob::try_new(oversized.clone(), Vec::new());
        let private_result = SealedBlob::try_new(Vec::new(), oversized);

        // ASSERT
        assert!(public_result.is_err(), "oversized public data should fail");
        assert!(
            private_result.is_err(),
            "oversized private data should fail"
        );
    }

    #[test]
    fn deserialize_rejects_invalid_size_slice() {
        // ARRANGE
        let invalid = [0_u8];

        // ACT
        let result = u16_from_le_slice(&invalid);

        // ASSERT
        assert!(result.is_err(), "invalid size slice should fail");
    }
}
