//! RSA-2048 PKCS#1 v1.5 signer for UEFI Secure Boot compatibility.

use core::result::Result as CoreResult;

use const_oid::db::rfc5912::SHA_256_WITH_RSA_ENCRYPTION;
use der::asn1::{Any, BitString};
use getrandom::SysRng;
use getrandom::rand_core::UnwrapErr;
use rsa::pkcs1v15::SigningKey;
use rsa::pkcs8::{DecodePrivateKey as _, EncodePrivateKey as _};
use rsa::{RsaPrivateKey, RsaPublicKey};
use sha2::{Digest as _, Sha256};
use signature::DigestSigner as _;
use signature::SignatureEncoding as _;
use spki::AlgorithmIdentifierOwned;
use zeroize::Zeroizing;

use crate::error::{Result, SboltError};

/// RSA key size in bits required for UEFI Secure Boot.
const KEY_SIZE_BITS: usize = 2048;

/// PKCS#1 v1.5 signer over SHA-256 for UEFI Secure Boot key operations.
pub struct Signer {
    signing_key: SigningKey<Sha256>,
}

impl Signer {
    /// Generate a new RSA-2048 key pair.
    ///
    /// # Errors
    ///
    /// Returns an error if RSA key generation fails.
    pub fn generate() -> Result<Self> {
        let signing_key = SigningKey::<Sha256>::random(&mut UnwrapErr(SysRng), KEY_SIZE_BITS)
            .map_err(|e| SboltError::KeyGeneration(format!("RSA key generation failed: {e}")))?;

        Ok(Self { signing_key })
    }

    /// Load from PKCS#8 DER bytes.
    ///
    /// # Errors
    ///
    /// Returns an error if the PKCS#8 document cannot be decoded as an RSA key.
    pub fn from_pkcs8_der(pkcs8_der: &[u8]) -> Result<Self> {
        let key = RsaPrivateKey::from_pkcs8_der(pkcs8_der)
            .map_err(|e| SboltError::KeyGeneration(format!("failed to load RSA key: {e}")))?;
        let signing_key = SigningKey::new(key);

        Ok(Self { signing_key })
    }

    /// Encode the private key as PKCS#8 DER.
    ///
    /// # Errors
    ///
    /// Returns an error if PKCS#8 encoding fails.
    pub fn to_pkcs8_der(&self) -> Result<Zeroizing<Vec<u8>>> {
        self.signing_key
            .as_ref()
            .to_pkcs8_der()
            .map(|doc| Zeroizing::new(doc.to_bytes().to_vec()))
            .map_err(|e| SboltError::KeyGeneration(format!("PKCS#8 encoding failed: {e}")))
    }

    /// Get the public key.
    #[must_use]
    pub fn public_key(&self) -> RsaPublicKey {
        self.signing_key.as_ref().to_public_key()
    }

    /// Sign data with RSA-PKCS1-v1.5 over SHA-256.
    ///
    /// # Errors
    ///
    /// Returns an error if RSA signing fails.
    pub fn sign_pkcs1v15_sha256(&self, data: &[u8]) -> Result<Vec<u8>> {
        self.signing_key
            .try_sign_digest(|digest| {
                digest.update(data);
                Ok(())
            })
            .map(|signature| signature.to_vec())
            .map_err(|e| SboltError::Signing(format!("RSA signing failed: {e}")))
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
    /// RSA-2048 signature size in bytes (`KEY_SIZE_BITS / 8`).
    const KEY_SIZE_BYTES: usize = 256;

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
        assert_eq!(
            KEY_SIZE_BYTES,
            signature.len(),
            "RSA-2048 signature must be 256 bytes"
        );
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
}
