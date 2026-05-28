//! ECDSA signer implementation wrapping ring for use with x509-cert.

extern crate alloc;

use alloc::vec::Vec;
use core::result;

use const_oid::ObjectIdentifier;
use const_oid::db::rfc5912::{ECDSA_WITH_SHA_256, SECP_256_R_1};
use der::asn1::{Any, BitString};
use ring::rand::SystemRandom;
use ring::signature::{ECDSA_P256_SHA256_ASN1_SIGNING, EcdsaKeyPair, KeyPair as _};
use spki::AlgorithmIdentifierOwned;
use zeroize::Zeroizing;

use crate::error::{PkiError, Result};

const ECDSA_WITH_SHA256_OID: ObjectIdentifier = ECDSA_WITH_SHA_256;
const EC_PUBLIC_KEY_OID: ObjectIdentifier = ObjectIdentifier::new_unwrap("1.2.840.10045.2.1");
const SECP256R1_OID: ObjectIdentifier = SECP_256_R_1;

/// Wrapper around `ring`'s `EcdsaKeyPair` that implements `RustCrypto` traits.
pub struct Signer {
    key_pair: EcdsaKeyPair,
    pkcs8_der: Zeroizing<Vec<u8>>,
    rng: SystemRandom,
}

impl Signer {
    /// Creates a new ECDSA signer by generating a fresh P-256 key pair.
    ///
    /// # Errors
    ///
    /// Returns an error if PKCS#8 key generation or key parsing fails.
    pub fn generate() -> Result<Self> {
        let rng = SystemRandom::new();
        let pkcs8_doc = EcdsaKeyPair::generate_pkcs8(&ECDSA_P256_SHA256_ASN1_SIGNING, &rng)
            .map_err(|_generation_error| PkiError::KeyGeneration)?;
        let pkcs8_der = Zeroizing::new(pkcs8_doc.as_ref().to_vec());

        EcdsaKeyPair::from_pkcs8(&ECDSA_P256_SHA256_ASN1_SIGNING, &pkcs8_der, &rng)
            .map_err(|_key_error| PkiError::InvalidKeyEncoding)
            .map(|key_pair| Self {
                key_pair,
                pkcs8_der,
                rng,
            })
    }

    /// Creates a signer from an existing PKCS#8 DER-encoded private key.
    ///
    /// # Errors
    ///
    /// Returns an error if the provided DER bytes do not encode a valid P-256
    /// PKCS#8 private key.
    pub fn from_pkcs8_der(pkcs8_der: &[u8]) -> Result<Self> {
        let rng = SystemRandom::new();
        EcdsaKeyPair::from_pkcs8(&ECDSA_P256_SHA256_ASN1_SIGNING, pkcs8_der, &rng)
            .map_err(|_key_error| PkiError::InvalidKeyEncoding)
            .map(|key_pair| Self {
                key_pair,
                pkcs8_der: Zeroizing::new(pkcs8_der.to_vec()),
                rng,
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
        self.key_pair.public_key().as_ref()
    }
}

/// Public key wrapper for the ring ECDSA signer.
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
            public_key_bytes: self.public_key_bytes().to_vec(),
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
        self.key_pair
            .sign(&self.rng, msg)
            .map(|sig| Signature(sig.as_ref().to_vec()))
            .map_err(|_sign_error| signature::Error::new())
    }
}
