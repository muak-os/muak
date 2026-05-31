//! RSA-2048 signer implementation for UEFI Secure Boot compatibility.

use core::result::Result as CoreResult;

use const_oid::db::rfc5912::SHA_256_WITH_RSA_ENCRYPTION;
use der::asn1::{Any, BitString};
use ring::digest::{Context, SHA256};
use ring::error::Unspecified;
use ring::rand::{SecureRandom as _, SystemRandom};
use rsa::pkcs1v15::Pkcs1v15Sign;
use rsa::pkcs8::{DecodePrivateKey as _, EncodePrivateKey as _};
use rsa::rand_core::{TryCryptoRng, TryRng, UnwrapErr};
use rsa::traits::SignatureScheme as _;
use rsa::{RsaPrivateKey, RsaPublicKey};
use spki::AlgorithmIdentifierOwned;

use crate::error::{Result, SboltError};

/// `DigestInfo` prefix for SHA-256 (DER-encoded `AlgorithmIdentifier` + OCTET STRING header).
const SHA256_DIGEST_INFO_PREFIX: &[u8] = &[
    0x30, 0x31, // SEQUENCE, 49 bytes
    0x30, 0x0d, // SEQUENCE, 13 bytes (AlgorithmIdentifier)
    0x06, 0x09, // OID, 9 bytes
    0x60, 0x86, 0x48, 0x01, 0x65, 0x03, 0x04, 0x02, 0x01, // SHA-256 OID
    0x05, 0x00, // NULL
    0x04, 0x20, // OCTET STRING, 32 bytes
];

/// Wrapper around ring's `SystemRandom` implementing `TryRng`/`TryCryptoRng`.
struct RingRng(SystemRandom);

impl TryRng for RingRng {
    type Error = Unspecified;

    fn try_next_u32(&mut self) -> CoreResult<u32, Self::Error> {
        let mut buf = [0_u8; 4];
        self.0.fill(&mut buf)?;
        Ok(u32::from_le_bytes(buf))
    }

    fn try_next_u64(&mut self) -> CoreResult<u64, Self::Error> {
        let mut buf = [0_u8; 8];
        self.0.fill(&mut buf)?;
        Ok(u64::from_le_bytes(buf))
    }

    fn try_fill_bytes(&mut self, dst: &mut [u8]) -> CoreResult<(), Self::Error> {
        self.0.fill(dst)
    }
}

impl TryCryptoRng for RingRng {}

/// Returns a `CryptoRng`-compatible RNG backed by ring's `SystemRandom`.
fn make_rng() -> UnwrapErr<RingRng> {
    UnwrapErr(RingRng(SystemRandom::new()))
}

/// Signer for UEFI Secure Boot RSA-2048 key operations.
pub struct Signer {
    private_key: RsaPrivateKey,
}

impl Signer {
    /// Generate a new RSA-2048 key pair.
    ///
    /// # Errors
    ///
    /// Returns an error if RSA key generation fails.
    pub fn generate() -> Result<Self> {
        let mut rng = make_rng();
        let private_key = RsaPrivateKey::new(&mut rng, 2048)
            .map_err(|e| SboltError::KeyGeneration(format!("RSA key generation failed: {e}")))?;

        Ok(Self { private_key })
    }

    /// Load from PKCS#8 DER bytes.
    ///
    /// # Errors
    ///
    /// Returns an error if the PKCS#8 document cannot be decoded as an RSA key.
    pub fn from_pkcs8_der(pkcs8_der: &[u8]) -> Result<Self> {
        let private_key = RsaPrivateKey::from_pkcs8_der(pkcs8_der)
            .map_err(|e| SboltError::KeyGeneration(format!("failed to load RSA key: {e}")))?;

        Ok(Self { private_key })
    }

    /// Encode the private key as PKCS#8 DER.
    ///
    /// # Errors
    ///
    /// Returns an error if PKCS#8 encoding fails.
    pub fn to_pkcs8_der(&self) -> Result<zeroize::Zeroizing<Vec<u8>>> {
        self.private_key
            .to_pkcs8_der()
            .map(|doc| zeroize::Zeroizing::new(doc.to_bytes().to_vec()))
            .map_err(|e| SboltError::KeyGeneration(format!("PKCS#8 encoding failed: {e}")))
    }

    /// Get the public key.
    #[must_use]
    pub fn public_key(&self) -> RsaPublicKey {
        self.private_key.to_public_key()
    }

    /// Sign data with RSA-PKCS1-SHA256 using ring for hashing.
    ///
    /// # Errors
    ///
    /// Returns an error if RSA signing fails.
    pub fn sign_pkcs1v15_sha256(&self, data: &[u8]) -> Result<Vec<u8>> {
        let mut ctx = Context::new(&SHA256);
        ctx.update(data);
        let digest = ctx.finish();

        let scheme = Pkcs1v15Sign {
            hash_len: Some(32),
            prefix: SHA256_DIGEST_INFO_PREFIX.into(),
        };

        let signature = scheme
            .sign::<UnwrapErr<RingRng>>(None, &self.private_key, digest.as_ref())
            .map_err(|e| SboltError::Signing(format!("RSA signing failed: {e}")))?;

        Ok(signature)
    }
}

impl signature::Keypair for Signer {
    type VerifyingKey = RsaPublicKey;

    fn verifying_key(&self) -> Self::VerifyingKey {
        self.public_key()
    }
}

impl spki::DynSignatureAlgorithmIdentifier for Signer {
    fn signature_algorithm_identifier(&self) -> spki::Result<AlgorithmIdentifierOwned> {
        Ok(AlgorithmIdentifierOwned {
            oid: SHA_256_WITH_RSA_ENCRYPTION,
            parameters: Some(Any::null()),
        })
    }
}

/// Signature wrapper for x509-cert RSA-PKCS1-SHA256 compatibility.
pub struct Signature(pub Vec<u8>);

impl spki::SignatureBitStringEncoding for Signature {
    fn to_bitstring(&self) -> der::Result<BitString> {
        BitString::from_bytes(&self.0)
    }
}

impl signature::Signer<Signature> for Signer {
    fn try_sign(&self, msg: &[u8]) -> CoreResult<Signature, signature::Error> {
        self.sign_pkcs1v15_sha256(msg)
            .map(Signature)
            .map_err(|_signing_error| signature::Error::new())
    }
}

#[cfg(test)]
mod tests {
    use rsa::traits::PublicKeyParts as _;
    use signature::{Keypair as _, Signer as _};
    use spki::DynSignatureAlgorithmIdentifier as _;
    use spki::SignatureBitStringEncoding as _;

    use super::*;

    #[test]
    fn signer_generates_serializes_and_reloads() {
        // ARRANGE
        let signer = Signer::generate().expect("generate signer");

        // ACT
        let pkcs8 = signer.to_pkcs8_der().expect("encode PKCS#8");
        let reloaded = Signer::from_pkcs8_der(&pkcs8).expect("reload PKCS#8");

        // ASSERT
        assert_eq!(signer.verifying_key().n(), reloaded.verifying_key().n());
        assert_eq!(signer.verifying_key().e(), reloaded.verifying_key().e());
    }

    #[test]
    fn signer_produces_signature_and_bitstring() {
        // ARRANGE
        let signer = Signer::generate().expect("generate signer");
        let message = b"test message";

        // ACT
        let signature = signer.sign_pkcs1v15_sha256(message).expect("sign message");
        let wrapped = signer
            .try_sign(message)
            .expect("signature trait signs message");
        let bit_string = wrapped.to_bitstring().expect("encode bit string");

        // ASSERT
        assert!(!signature.is_empty());
        assert_eq!(signature, wrapped.0);
        assert_eq!(bit_string.raw_bytes(), signature.as_slice());
    }

    #[test]
    fn from_pkcs8_der_rejects_invalid_key() {
        // ARRANGE
        let invalid = b"not-a-private-key";

        // ACT
        let result = Signer::from_pkcs8_der(invalid);

        // ASSERT
        result.err().expect("invalid key should fail");
    }

    #[test]
    fn signature_algorithm_identifier_uses_sha256_rsa() {
        // ARRANGE
        let signer = Signer::generate().expect("generate signer");

        // ACT
        let identifier = signer
            .signature_algorithm_identifier()
            .expect("signature algorithm identifier");

        // ASSERT
        assert_eq!(identifier.oid, SHA_256_WITH_RSA_ENCRYPTION);
        assert_eq!(identifier.parameters, Some(Any::null()));
    }

    #[test]
    fn ring_rng_generates_random_words() {
        // ARRANGE
        let mut rng = RingRng(SystemRandom::new());
        let mut bytes = [0_u8; 16];

        // ACT
        let word32 = rng.try_next_u32().expect("generate u32");
        let word64 = rng.try_next_u64().expect("generate u64");
        rng.try_fill_bytes(&mut bytes).expect("fill bytes");

        // ASSERT
        let any_bytes = bytes.iter().any(|byte| *byte != 0);
        assert!(word32 != 0 || word64 != 0 || any_bytes);
    }
}
