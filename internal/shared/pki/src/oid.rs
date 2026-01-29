//! OID constants for cryptographic algorithms.

/// OID for ECDSA with SHA-256 signature algorithm
pub const ECDSA_WITH_SHA256_OID: const_oid::ObjectIdentifier =
    const_oid::db::rfc5912::ECDSA_WITH_SHA_256;

/// OID for EC public key algorithm
pub const EC_PUBLIC_KEY_OID: const_oid::ObjectIdentifier =
    const_oid::ObjectIdentifier::new_unwrap("1.2.840.10045.2.1");

/// OID for P-256 curve (secp256r1)
pub const SECP256R1_OID: const_oid::ObjectIdentifier = const_oid::db::rfc5912::SECP_256_R_1;
