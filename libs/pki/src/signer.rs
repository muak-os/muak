//! ECDSA signer implementation wrapping ring for use with x509-cert.

extern crate alloc;

use alloc::vec::Vec;

use der::asn1::BitString;
use ring::{
    rand::SystemRandom,
    signature::{ECDSA_P256_SHA256_ASN1_SIGNING, EcdsaKeyPair, KeyPair as RingKeyPair},
};
use spki::AlgorithmIdentifierOwned;

use crate::error::{Error, Result};
use crate::oid::{EC_PUBLIC_KEY_OID, ECDSA_WITH_SHA256_OID, SECP256R1_OID};

/// Wrapper around ring's EcdsaKeyPair that implements RustCrypto traits.
pub struct RingEcdsaSigner {
    key_pair: EcdsaKeyPair,
    pkcs8_der: Vec<u8>,
    rng: SystemRandom,
}

impl RingEcdsaSigner {
    /// Creates a new ECDSA signer by generating a fresh P-256 key pair.
    pub fn generate() -> Result<Self> {
        let rng = SystemRandom::new();
        let pkcs8_doc = EcdsaKeyPair::generate_pkcs8(&ECDSA_P256_SHA256_ASN1_SIGNING, &rng)
            .map_err(|_| Error::KeyGeneration)?;
        let pkcs8_der = pkcs8_doc.as_ref().to_vec();
        let key_pair = EcdsaKeyPair::from_pkcs8(&ECDSA_P256_SHA256_ASN1_SIGNING, &pkcs8_der, &rng)
            .map_err(|_| Error::InvalidKeyEncoding)?;
        Ok(Self {
            key_pair,
            pkcs8_der,
            rng,
        })
    }

    /// Creates a signer from an existing PKCS#8 DER-encoded private key.
    pub fn from_pkcs8_der(pkcs8_der: &[u8]) -> Result<Self> {
        let rng = SystemRandom::new();
        let key_pair = EcdsaKeyPair::from_pkcs8(&ECDSA_P256_SHA256_ASN1_SIGNING, pkcs8_der, &rng)
            .map_err(|_| Error::InvalidKeyEncoding)?;
        Ok(Self {
            key_pair,
            pkcs8_der: pkcs8_der.to_vec(),
            rng,
        })
    }

    /// Returns the PKCS#8 DER-encoded private key.
    pub fn pkcs8_der(&self) -> &[u8] {
        &self.pkcs8_der
    }

    /// Returns the public key in uncompressed point format.
    pub fn public_key_bytes(&self) -> &[u8] {
        self.key_pair.public_key().as_ref()
    }
}

/// Verifying key (public key) for the ring ECDSA signer.
#[derive(Clone)]
pub struct RingEcdsaVerifyingKey {
    public_key_bytes: Vec<u8>,
}

impl spki::EncodePublicKey for RingEcdsaVerifyingKey {
    fn to_public_key_der(&self) -> spki::Result<spki::Document> {
        use der::Encode;

        let algorithm = spki::AlgorithmIdentifier {
            oid: EC_PUBLIC_KEY_OID,
            parameters: Some(der::asn1::Any::from(&SECP256R1_OID)),
        };
        let spki = spki::SubjectPublicKeyInfo {
            algorithm,
            subject_public_key: BitString::from_bytes(&self.public_key_bytes)
                .map_err(|_| spki::Error::KeyMalformed)?,
        };
        let der_bytes = spki.to_der().map_err(|_| spki::Error::KeyMalformed)?;
        spki::Document::try_from(der_bytes).map_err(|_| spki::Error::KeyMalformed)
    }
}

impl signature::Keypair for RingEcdsaSigner {
    type VerifyingKey = RingEcdsaVerifyingKey;

    fn verifying_key(&self) -> Self::VerifyingKey {
        RingEcdsaVerifyingKey {
            public_key_bytes: self.public_key_bytes().to_vec(),
        }
    }
}

impl spki::DynSignatureAlgorithmIdentifier for RingEcdsaSigner {
    fn signature_algorithm_identifier(&self) -> spki::Result<AlgorithmIdentifierOwned> {
        Ok(AlgorithmIdentifierOwned {
            oid: ECDSA_WITH_SHA256_OID,
            parameters: None,
        })
    }
}

/// ECDSA signature wrapper for x509-cert.
pub struct EcdsaSignature(pub Vec<u8>);

impl spki::SignatureBitStringEncoding for EcdsaSignature {
    fn to_bitstring(&self) -> der::Result<BitString> {
        BitString::from_bytes(&self.0)
    }
}

impl signature::Signer<EcdsaSignature> for RingEcdsaSigner {
    fn try_sign(&self, msg: &[u8]) -> std::result::Result<EcdsaSignature, signature::Error> {
        let sig = self
            .key_pair
            .sign(&self.rng, msg)
            .map_err(|_| signature::Error::new())?;
        Ok(EcdsaSignature(sig.as_ref().to_vec()))
    }
}
