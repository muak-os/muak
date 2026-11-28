//! Sealed blob storage format.

use crate::buffer::u16_len;
use crate::error::{Result, Tpm2Error};

/// Maximum encoded public area size of a sealed object.
///
/// The keyedhash public template written by `commands/template.rs` is 46 bytes,
/// and the TPM appends its unique digest to it, so the encoded public area stays
/// close to that size. The private area carries a 32-byte integrity value plus
/// the AES-CFB encrypted sensitive area. Capping each part at 256 bytes leaves a
/// wide margin for a 64-byte sealed secret while keeping a `Load` command far
/// below `MAX_COMMAND_SIZE`; a TPM that returns a larger part fails loudly with
/// `Tpm2Error::BufferTooLarge`.
const MAX_PUBLIC_SIZE: usize = 256;

/// Maximum encoded private area size of a sealed object.
const MAX_PRIVATE_SIZE: usize = 256;

/// Sealed blob format: `[pub_size:u16][pub_data][priv_size:u16][priv_data]`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Sealed {
    public: [u8; MAX_PUBLIC_SIZE],
    public_len: usize,
    private: [u8; MAX_PRIVATE_SIZE],
    private_len: usize,
}

impl Sealed {
    /// Maximum serialized size in bytes, for callers sizing a stack buffer.
    pub const MAX_SERIALIZED_SIZE: usize = 2 + MAX_PUBLIC_SIZE + 2 + MAX_PRIVATE_SIZE;

    /// Creates a validated sealed blob, copying both parts into fixed storage.
    ///
    /// # Errors
    ///
    /// Returns `Tpm2Error::BufferTooLarge` naming the part cap when either part
    /// exceeds `MAX_PUBLIC_SIZE` or `MAX_PRIVATE_SIZE`.
    pub fn try_new(public: &[u8], private: &[u8]) -> Result<Self> {
        let (public, public_len) = copy_part::<MAX_PUBLIC_SIZE>(public)?;
        let (private, private_len) = copy_part::<MAX_PRIVATE_SIZE>(private)?;

        Ok(Self {
            public,
            public_len,
            private,
            private_len,
        })
    }

    /// Serializes into the caller buffer, returning the written prefix.
    ///
    /// # Errors
    ///
    /// Returns `Tpm2Error::OutputTooSmall` when the buffer cannot hold the whole
    /// blob; a partial blob is never written.
    #[must_use = "callers need the written prefix to store the blob"]
    pub fn serialize_into<'a>(&self, out: &'a mut [u8]) -> Result<&'a [u8]> {
        // Both lengths are capped, so the sum always fits the maximum serialized size.
        let needed = self
            .public_len
            .saturating_add(self.private_len)
            .saturating_add(4);
        let Some(target) = out.get_mut(..needed) else {
            return Err(Tpm2Error::OutputTooSmall {
                needed,
                provided: out.len(),
            });
        };
        // Both conversions happen before any write, so a failure cannot leave stale sizes.
        let public_len_field = u16_len(self.public_len)?.to_le_bytes();
        let private_len_field = u16_len(self.private_len)?.to_le_bytes();
        let (public_len_slot, rest) = target.split_at_mut(2);
        let (public_slot, rest) = rest.split_at_mut(self.public_len);
        let (private_len_slot, private_slot) = rest.split_at_mut(2);
        public_len_slot.copy_from_slice(&public_len_field);
        public_slot.copy_from_slice(self.public());
        private_len_slot.copy_from_slice(&private_len_field);
        private_slot.copy_from_slice(self.private());

        Ok(target)
    }

    /// Deserializes from wire format, ignoring trailing data.
    ///
    /// # Errors
    ///
    /// Returns an error if the data does not contain a complete sealed blob, or
    /// if either part exceeds its cap.
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

        Self::try_new(pub_data, priv_data)
    }

    /// Public area bytes of the sealed object.
    #[must_use = "callers need the public area to load the object"]
    pub(crate) fn public(&self) -> &[u8] {
        // The length is only ever set from a copy that fits the array cap.
        self.public.get(..self.public_len).unwrap_or_default()
    }

    /// Private area bytes of the sealed object.
    #[must_use = "callers need the private area to unseal the object"]
    pub(crate) fn private(&self) -> &[u8] {
        // The length is only ever set from a copy that fits the array cap.
        self.private.get(..self.private_len).unwrap_or_default()
    }
}

fn copy_part<const N: usize>(source: &[u8]) -> Result<([u8; N], usize)> {
    let mut part = [0_u8; N];
    let Some(target) = part.get_mut(..source.len()) else {
        return Err(Tpm2Error::BufferTooLarge {
            actual: source.len(),
            max: N,
        });
    };
    target.copy_from_slice(source);

    Ok((part, source.len()))
}

const fn u16_from_le_slice(bytes: &[u8]) -> Result<u16> {
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
        let blob = Sealed::try_new(&[1, 2, 3, 4, 5], &[10, 20, 30]).expect("blob should be valid");
        let mut wire = [0_u8; Sealed::MAX_SERIALIZED_SIZE];

        // ACT
        let wire = blob
            .serialize_into(&mut wire)
            .expect("blob should serialize");
        let deserialized = Sealed::deserialize(wire);

        // ASSERT
        assert_eq!(wire.len(), 12, "wire should hold both sized parts");
        assert_eq!(
            wire.get(..2),
            Some(5_u16.to_le_bytes().as_slice()),
            "public size should be little endian"
        );
        let deserialized = deserialized.expect("blob should deserialize");
        assert_eq!(deserialized, blob, "roundtrip should preserve the blob");
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
    fn sealed_blob_empty_roundtrip() {
        // ARRANGE
        let blob = Sealed::try_new(&[], &[]).expect("empty blob should be valid");
        let mut wire = [0_u8; Sealed::MAX_SERIALIZED_SIZE];

        // ACT
        let serialized = blob
            .serialize_into(&mut wire)
            .expect("empty blob should serialize");
        let deserialized = Sealed::deserialize(serialized);

        // ASSERT
        assert_eq!(serialized.len(), 4, "empty blob should keep both sizes");
        let deserialized = deserialized.expect("blob should deserialize");
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
    fn full_parts_serialize_into_max_serialized_size() {
        // ARRANGE
        let public = [0xAA_u8; MAX_PUBLIC_SIZE];
        let private = [0xBB_u8; MAX_PRIVATE_SIZE];
        let blob = Sealed::try_new(&public, &private).expect("capped parts should be valid");
        let mut wire = [0_u8; Sealed::MAX_SERIALIZED_SIZE];

        // ACT
        let serialized = blob.serialize_into(&mut wire);

        // ASSERT
        let serialized = serialized.expect("full parts should serialize");
        assert_eq!(
            serialized.len(),
            Sealed::MAX_SERIALIZED_SIZE,
            "full parts should fill the maximum serialized size"
        );
        assert_eq!(
            serialized.get(2..2 + MAX_PUBLIC_SIZE),
            Some(public.as_slice()),
            "public part should be copied unchanged"
        );
        assert_eq!(
            serialized.get(4 + MAX_PUBLIC_SIZE..),
            Some(private.as_slice()),
            "private part should be copied unchanged"
        );
    }

    #[test]
    fn serialize_into_rejects_short_output_buffer() {
        // ARRANGE
        let blob = Sealed::try_new(&[1, 2, 3, 4, 5], &[10, 20, 30]).expect("blob should be valid");
        let mut wire = [0_u8; 11];

        // ACT
        let result = blob.serialize_into(&mut wire);

        // ASSERT
        assert!(
            matches!(
                result,
                Err(Tpm2Error::OutputTooSmall {
                    needed: 12,
                    provided: 11
                })
            ),
            "short buffer should report the needed and provided sizes"
        );
        assert_eq!(wire, [0_u8; 11], "short buffer should stay untouched");
    }

    #[test]
    fn accessors_return_original_slices() {
        // ARRANGE
        let blob = Sealed::try_new(&[1, 2], &[3, 4]).expect("small blob should be valid");

        // ACT
        let public = blob.public();
        let private = blob.private();

        // ASSERT
        assert_eq!(
            public,
            &[1, 2],
            "public accessor should expose public bytes"
        );
        assert_eq!(
            private,
            &[3, 4],
            "private accessor should expose private bytes"
        );
    }

    #[test]
    fn sealed_blob_invalid() {
        // ACT & ASSERT
        assert!(Sealed::deserialize(&[]).is_err(), "empty blob should fail");
        assert!(
            Sealed::deserialize(&[0, 0]).is_err(),
            "missing private size should fail"
        );
        assert!(
            Sealed::deserialize(&[5, 0, 1]).is_err(),
            "truncated public data should fail"
        );
    }

    #[test]
    fn sealed_blob_rejects_truncated_private_data() {
        // ARRANGE
        let data = [1, 0, 0xAA, 2, 0, 0xBB];

        // ACT
        let result = Sealed::deserialize(&data);

        // ASSERT
        assert!(result.is_err(), "truncated private data should fail");
    }

    #[test]
    fn sealed_blob_allows_trailing_data() {
        // ARRANGE
        let data = [1, 0, 0xAA, 1, 0, 0xBB, 0xCC];

        // ACT
        let result = Sealed::deserialize(&data);

        // ASSERT
        let blob = result.expect("blob should deserialize");
        assert_eq!(blob.public(), &[0xAA], "public data should match");
        assert_eq!(blob.private(), &[0xBB], "private data should match");
    }

    #[test]
    fn try_new_rejects_oversized_parts() {
        // ARRANGE
        let oversized_public = [0_u8; MAX_PUBLIC_SIZE + 1];
        let oversized_private = [0_u8; MAX_PRIVATE_SIZE + 1];

        // ACT
        let public_result = Sealed::try_new(&oversized_public, &[]);
        let private_result = Sealed::try_new(&[], &oversized_private);

        // ASSERT
        assert!(
            matches!(
                public_result,
                Err(Tpm2Error::BufferTooLarge { actual, max })
                    if actual == MAX_PUBLIC_SIZE + 1 && max == MAX_PUBLIC_SIZE
            ),
            "oversized public data should report the public cap"
        );
        assert!(
            matches!(
                private_result,
                Err(Tpm2Error::BufferTooLarge { actual, max })
                    if actual == MAX_PRIVATE_SIZE + 1 && max == MAX_PRIVATE_SIZE
            ),
            "oversized private data should report the private cap"
        );
    }

    #[test]
    fn deserialize_rejects_part_larger_than_cap() {
        // ARRANGE
        let declared = u16::try_from(MAX_PUBLIC_SIZE + 1).expect("cap should fit u16");
        let mut data = [0_u8; 2 + MAX_PUBLIC_SIZE + 1 + 2];
        data.get_mut(..2)
            .expect("public size range should exist")
            .copy_from_slice(&declared.to_le_bytes());

        // ACT
        let result = Sealed::deserialize(&data);

        // ASSERT
        assert!(
            matches!(result, Err(Tpm2Error::BufferTooLarge { max, .. }) if max == MAX_PUBLIC_SIZE),
            "public part above the cap should be rejected"
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

    #[test]
    fn deserialize_rejects_missing_private_size_bytes() {
        // ARRANGE
        let data = [0, 0, 1];

        // ACT
        let result = Sealed::deserialize(&data);

        // ASSERT
        assert!(result.is_err(), "missing private size bytes should fail");
    }
}
