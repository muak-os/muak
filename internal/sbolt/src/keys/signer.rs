//! RSA-2048 signer implementation for UEFI Secure Boot compatibility

use std::convert::Infallible;

use der::asn1::BitString;
use ring::digest::{Context, SHA256};
use ring::rand::{SecureRandom, SystemRandom};
use rsa::pkcs1v15::Pkcs1v15Sign;
use rsa::pkcs8::{DecodePrivateKey, EncodePrivateKey};
use rsa::rand_core;
use rsa::traits::SignatureScheme;
use rsa::{RsaPrivateKey, RsaPublicKey};
use spki::AlgorithmIdentifierOwned;

use crate::{Error, Result};

/// DigestInfo prefix for SHA-256 (DER-encoded AlgorithmIdentifier + OCTET STRING header).
///
/// Format: SEQUENCE { AlgorithmIdentifier { OID, NULL }, OCTET STRING (32 bytes) }
const SHA256_DIGEST_INFO_PREFIX: &[u8] = &[
    0x30, 0x31, // SEQUENCE, 49 bytes
    0x30, 0x0d, // SEQUENCE, 13 bytes (AlgorithmIdentifier)
    0x06, 0x09, // OID, 9 bytes
    0x60, 0x86, 0x48, 0x01, 0x65, 0x03, 0x04, 0x02, 0x01, // SHA-256 OID
    0x05, 0x00, // NULL
    0x04, 0x20, // OCTET STRING, 32 bytes
];

/// Wrapper around ring's SystemRandom to implement rand_core traits.
struct RingRng(SystemRandom);

impl rand_core::TryRng for RingRng {
    type Error = Infallible;

    fn try_next_u32(&mut self) -> std::result::Result<u32, Self::Error> {
        let mut buf = [0u8; 4];
        self.0.fill(&mut buf).expect("RNG failure");
        Ok(u32::from_le_bytes(buf))
    }

    fn try_next_u64(&mut self) -> std::result::Result<u64, Self::Error> {
        let mut buf = [0u8; 8];
        self.0.fill(&mut buf).expect("RNG failure");
        Ok(u64::from_le_bytes(buf))
    }

    fn try_fill_bytes(&mut self, dest: &mut [u8]) -> std::result::Result<(), Self::Error> {
        self.0.fill(dest).expect("RNG failure");
        Ok(())
    }
}

impl rand_core::TryCryptoRng for RingRng {}

/// RSA-2048 signer for UEFI Secure Boot key operations.
pub struct Rsa2048Signer {
    private_key: RsaPrivateKey,
}

impl Rsa2048Signer {
    /// Generate a new RSA-2048 key pair.
    pub fn generate() -> Result<Self> {
        let mut rng = RingRng(SystemRandom::new());
        let private_key = RsaPrivateKey::new(&mut rng, 2048)
            .map_err(|e| Error::KeyGeneration(format!("RSA key generation failed: {e}")))?;

        Ok(Self { private_key })
    }

    /// Load from PKCS#8 DER bytes.
    pub fn from_pkcs8_der(pkcs8_der: &[u8]) -> Result<Self> {
        let private_key = RsaPrivateKey::from_pkcs8_der(pkcs8_der)
            .map_err(|e| Error::KeyGeneration(format!("failed to load RSA key: {e}")))?;

        Ok(Self { private_key })
    }

    /// Encode the private key as PKCS#8 DER.
    pub fn to_pkcs8_der(&self) -> Result<Vec<u8>> {
        self.private_key
            .to_pkcs8_der()
            .map(|doc| doc.to_bytes().to_vec())
            .map_err(|e| Error::KeyGeneration(format!("PKCS#8 encoding failed: {e}")))
    }

    /// Get the public key.
    pub fn public_key(&self) -> RsaPublicKey {
        self.private_key.to_public_key()
    }

    /// Sign data with RSA-PKCS1-SHA256 using ring for hashing.
    pub fn sign(&self, data: &[u8]) -> Result<Vec<u8>> {
        let mut ctx = Context::new(&SHA256);
        ctx.update(data);
        let digest = ctx.finish();

        let scheme = Pkcs1v15Sign {
            hash_len: Some(32),
            prefix: SHA256_DIGEST_INFO_PREFIX.into(),
        };

        let signature = scheme
            .sign::<RingRng>(None, &self.private_key, digest.as_ref())
            .map_err(|e| Error::Signing(format!("RSA signing failed: {e}")))?;

        Ok(signature)
    }
}

impl signature::Keypair for Rsa2048Signer {
    type VerifyingKey = RsaPublicKey;

    fn verifying_key(&self) -> Self::VerifyingKey {
        self.public_key()
    }
}

impl spki::DynSignatureAlgorithmIdentifier for Rsa2048Signer {
    fn signature_algorithm_identifier(&self) -> spki::Result<AlgorithmIdentifierOwned> {
        Ok(AlgorithmIdentifierOwned {
            oid: const_oid::db::rfc5912::SHA_256_WITH_RSA_ENCRYPTION,
            parameters: Some(der::asn1::Any::null()),
        })
    }
}

/// RSA-PKCS1-SHA256 signature wrapper for x509-cert compatibility.
pub struct Rsa2048Signature(pub Vec<u8>);

impl spki::SignatureBitStringEncoding for Rsa2048Signature {
    fn to_bitstring(&self) -> der::Result<BitString> {
        BitString::from_bytes(&self.0)
    }
}

impl signature::Signer<Rsa2048Signature> for Rsa2048Signer {
    fn try_sign(&self, msg: &[u8]) -> std::result::Result<Rsa2048Signature, signature::Error> {
        self.sign(msg)
            .map(Rsa2048Signature)
            .map_err(|_| signature::Error::new())
    }
}
