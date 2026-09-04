//! ECDSA signer implementation wrapping `p256` for use with x509-cert.

extern crate alloc;

use alloc::vec::Vec;
use core::result;

use const_oid::ObjectIdentifier;
use const_oid::db::rfc5912::{ECDSA_WITH_SHA_256, SECP_256_R_1};
use der::asn1::{Any, BitString};
use getrandom::SysRng;
use p256::ecdsa::{Signature as EcdsaSignature, SigningKey};
use p256::elliptic_curve::Generate as _;
use p256::elliptic_curve::pkcs8::{DecodePrivateKey as _, EncodePrivateKey as _};
use p256::elliptic_curve::sec1::ToSec1Point as _;
use spki::AlgorithmIdentifierOwned;
use zeroize::Zeroizing;

use crate::error::{PkiError, Result};

const ECDSA_WITH_SHA256_OID: ObjectIdentifier = ECDSA_WITH_SHA_256;
const EC_PUBLIC_KEY_OID: ObjectIdentifier = ObjectIdentifier::new_unwrap("1.2.840.10045.2.1");
const SECP256R1_OID: ObjectIdentifier = SECP_256_R_1;

/// Uncompressed SEC1 point length for P-256 (`0x04 || X || Y`).
const UNCOMPRESSED_POINT_LEN: usize = 65;

/// Wrapper around `p256`'s `SigningKey` that implements `RustCrypto` traits.
pub struct Signer {
    key: SigningKey,
    pkcs8_der: Zeroizing<Vec<u8>>,
    public_key: Vec<u8>,
}

impl Signer {
    /// Creates a new ECDSA signer by generating a fresh P-256 key pair.
    ///
    /// # Errors
    ///
    /// Returns an error if PKCS#8 key encoding fails.
    pub fn generate() -> Result<Self> {
        let key = SigningKey::try_generate_from_rng(&mut SysRng)
            .map_err(|_error| PkiError::KeyGeneration)?;
        let document = key
            .to_pkcs8_der()
            .map_err(|_error| PkiError::KeyGeneration)?;
        let pkcs8_der = Zeroizing::new(document.as_bytes().to_vec());
        let public_key = Self::encode_public_key(&key);

        Ok(Self {
            key,
            pkcs8_der,
            public_key,
        })
    }

    /// Creates a signer from an existing PKCS#8 DER-encoded private key.
    ///
    /// # Errors
    ///
    /// Returns an error if the provided DER bytes do not encode a valid P-256
    /// PKCS#8 private key.
    pub fn from_pkcs8_der(pkcs8_der: &[u8]) -> Result<Self> {
        let key = SigningKey::from_pkcs8_der(pkcs8_der)
            .map_err(|_key_error| PkiError::InvalidKeyEncoding)?;
        let public_key = Self::encode_public_key(&key);

        Ok(Self {
            key,
            pkcs8_der: Zeroizing::new(pkcs8_der.to_vec()),
            public_key,
        })
    }

    /// Returns the PKCS#8 DER-encoded private key.
    #[must_use]
    pub fn pkcs8_der(&self) -> &[u8] {
        &self.pkcs8_der
    }

    /// Returns the public key in uncompressed point format.
    #[must_use]
    pub fn public_key_bytes(&self) -> &[u8] {
        &self.public_key
    }

    fn encode_public_key(key: &SigningKey) -> Vec<u8> {
        let point = key.verifying_key().as_affine().to_sec1_point(false);
        let bytes = point.as_bytes();
        debug_assert_eq!(
            bytes.len(),
            UNCOMPRESSED_POINT_LEN,
            "P-256 uncompressed SEC1 point must be 65 bytes"
        );

        bytes.to_vec()
    }
}

/// Public key wrapper for the ECDSA signer.
#[derive(Clone)]
pub struct Verifier {
    public_key_bytes: Vec<u8>,
}

impl Verifier {
    pub(crate) fn subject_public_key_info(&self) -> der::Result<spki::SubjectPublicKeyInfoOwned> {
        BitString::from_bytes(&self.public_key_bytes).map(|subject_public_key| {
            spki::SubjectPublicKeyInfo {
                algorithm: public_key_algorithm(),
                subject_public_key,
            }
        })
    }
}

impl spki::EncodePublicKey for Verifier {
    fn to_public_key_der(&self) -> spki::Result<spki::Document> {
        let spki = self.subject_public_key_info()?;
        spki::Document::try_from(&spki)
    }
}

fn public_key_algorithm() -> spki::AlgorithmIdentifier<Any> {
    spki::AlgorithmIdentifier {
        oid: EC_PUBLIC_KEY_OID,
        parameters: Some(Any::from(&SECP256R1_OID)),
    }
}

impl signature::Keypair for Signer {
    type VerifyingKey = Verifier;

    fn verifying_key(&self) -> Self::VerifyingKey {
        Verifier {
            public_key_bytes: self.public_key.clone(),
        }
    }
}

impl spki::DynSignatureAlgorithmIdentifier for Signer {
    fn signature_algorithm_identifier(&self) -> spki::Result<AlgorithmIdentifierOwned> {
        Ok(AlgorithmIdentifierOwned {
            oid: ECDSA_WITH_SHA256_OID,
            parameters: None,
        })
    }
}

/// ECDSA signature wrapper for x509-cert.
pub struct Signature(pub Vec<u8>);

impl spki::SignatureBitStringEncoding for Signature {
    fn to_bitstring(&self) -> der::Result<BitString> {
        BitString::from_bytes(&self.0)
    }
}

impl signature::Signer<Signature> for Signer {
    fn try_sign(&self, msg: &[u8]) -> result::Result<Signature, signature::Error> {
        let signature: EcdsaSignature = self.key.sign(msg);

        Ok(Signature(signature.to_der().as_ref().to_vec()))
    }
}

#[cfg(test)]
mod tests {
    use p256::ecdsa::VerifyingKey;
    use signature::{Signer as _, Verifier as _};

    use super::*;

    /// PKCS#8 DER of a P-256 key generated with OpenSSL.
    const COMPAT_PKCS8_DER: &[u8] = &[
        0x30, 0x81, 0x87, 0x02, 0x01, 0x00, 0x30, 0x13, 0x06, 0x07, 0x2a, 0x86, 0x48, 0xce, 0x3d,
        0x02, 0x01, 0x06, 0x08, 0x2a, 0x86, 0x48, 0xce, 0x3d, 0x03, 0x01, 0x07, 0x04, 0x6d, 0x30,
        0x6b, 0x02, 0x01, 0x01, 0x04, 0x20, 0x8a, 0xf8, 0xab, 0xf1, 0x47, 0x9e, 0x50, 0x21, 0x44,
        0x9a, 0x0a, 0x28, 0x00, 0x98, 0x78, 0x54, 0xd7, 0x35, 0xb1, 0xb2, 0x92, 0x9d, 0x24, 0x9b,
        0xe3, 0x54, 0x38, 0x0f, 0xea, 0x2c, 0x40, 0x85, 0xa1, 0x44, 0x03, 0x42, 0x00, 0x04, 0x4f,
        0x6c, 0x0e, 0xf4, 0xa4, 0x89, 0x0d, 0xae, 0xc3, 0x0d, 0xa1, 0x31, 0xc6, 0xf3, 0x41, 0x12,
        0x10, 0x55, 0x86, 0xb4, 0x57, 0x3f, 0x62, 0xc3, 0x89, 0xa3, 0x40, 0xbc, 0xb1, 0x1a, 0x20,
        0xcd, 0x26, 0x62, 0xac, 0x84, 0x4f, 0x17, 0xb1, 0xc4, 0x68, 0x1f, 0xa2, 0x8e, 0x1c, 0xc8,
        0xe0, 0xbf, 0x8c, 0x9b, 0xa6, 0xad, 0x48, 0x72, 0x08, 0x17, 0x25, 0x8a, 0x2e, 0x74, 0x8b,
        0x33, 0xd7, 0xc0,
    ];

    /// ASN.1 DER ECDSA signature over b"muak ring compat test vector" produced with
    /// `openssl dgst -sha256 -sign` (BoringSSL-lineage implementation, like `ring`).
    const COMPAT_OPENSSL_SIG_DER: &[u8] = &[
        0x30, 0x45, 0x02, 0x20, 0x20, 0xc5, 0x84, 0x4f, 0xa6, 0x58, 0x25, 0x86, 0x1d, 0x95, 0xaa,
        0xef, 0xec, 0x8a, 0x5b, 0xdd, 0x3e, 0xe7, 0x32, 0x1e, 0x53, 0x60, 0x22, 0xd5, 0x5d, 0x1c,
        0x3b, 0x33, 0xfe, 0xa5, 0xd2, 0xe3, 0x02, 0x21, 0x00, 0x87, 0xb2, 0xbb, 0x9d, 0x59, 0xd4,
        0x5b, 0x1b, 0x43, 0x25, 0xc1, 0xd8, 0xe3, 0x76, 0x6b, 0xce, 0x92, 0x54, 0x47, 0x36, 0x3e,
        0x3d, 0x1e, 0x75, 0x81, 0x1c, 0xed, 0x4e, 0xfa, 0x48, 0xdf, 0xf6,
    ];

    const COMPAT_MESSAGE: &[u8] = b"muak ring compat test vector";

    #[test]
    fn signer_generates_and_round_trips_pkcs8() {
        // ARRANGE
        let signer = Signer::generate().expect("generate signer");

        // ACT
        let pkcs8 = signer.pkcs8_der().to_vec();
        let reloaded = Signer::from_pkcs8_der(&pkcs8).expect("reload PKCS#8");

        // ASSERT
        assert_eq!(signer.public_key_bytes(), reloaded.public_key_bytes());
        assert_eq!(UNCOMPRESSED_POINT_LEN, signer.public_key_bytes().len());
    }

    #[test]
    fn verifier_accepts_openssl_signature() {
        // ARRANGE
        let signer = Signer::from_pkcs8_der(COMPAT_PKCS8_DER).expect("load compat key");
        let signature = EcdsaSignature::from_der(COMPAT_OPENSSL_SIG_DER)
            .expect("parse OpenSSL ASN.1 DER signature");
        let verifying_key = VerifyingKey::from_sec1_bytes(signer.public_key_bytes())
            .expect("parse public key point");

        // ACT
        let result = verifying_key.verify(COMPAT_MESSAGE, &signature);

        // ASSERT
        result.expect("OpenSSL-produced signature must verify (cross-implementation compat)");
    }

    #[test]
    fn signer_output_verifies_and_rejects_tampering() {
        // ARRANGE
        let signer = Signer::from_pkcs8_der(COMPAT_PKCS8_DER).expect("load compat key");

        // ACT
        let signature = signer
            .try_sign(COMPAT_MESSAGE)
            .expect("sign compat message");
        let verifying_key = VerifyingKey::from_sec1_bytes(signer.public_key_bytes())
            .expect("parse public key point");
        let parsed = EcdsaSignature::from_der(&signature.0).expect("parse ASN.1 DER signature");
        let tampered = EcdsaSignature::from_der(&{
            let mut bytes = signature.0.clone();
            let last = bytes.last_mut().expect("non-empty signature");
            *last ^= 0x01;
            bytes
        })
        .expect("parse tampered ASN.1 DER signature");

        // ASSERT
        verifying_key
            .verify(COMPAT_MESSAGE, &parsed)
            .expect("self-produced signature must verify");
        assert!(
            verifying_key.verify(COMPAT_MESSAGE, &tampered).is_err(),
            "tampered signature must not verify"
        );
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
}
