//! TPM2 token stored in the LUKS2 JSON metadata.

use std::collections::HashMap;

use base64ct::{Base64, Encoding as _};
use serde::{Deserialize, Serialize};

use crate::error::{Luks2Error, Result};

/// TPM2 token stored in LUKS2 metadata.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Tpm2Token {
    /// Token type discriminator.
    pub r#type: String,
    /// Keyslot identifiers bound to this token.
    pub keyslots: Vec<String>,
    /// PCR indices bound to the sealed blob.
    #[serde(rename = "tpm2-pcrs")]
    pub tpm2_pcrs: Vec<u32>,
    /// PCR bank hash algorithm name.
    #[serde(rename = "tpm2-hash-alg")]
    pub tpm2_hash_alg: String,
    /// Base64-encoded sealed key blob.
    #[serde(rename = "tpm2-blob")]
    pub tpm2_blob: String,
    /// Base64-encoded TPM2 policy digest.
    #[serde(rename = "tpm2-policy-hash")]
    pub tpm2_policy_hash: String,
}

impl Tpm2Token {
    /// Builds a token bound to the given PCR indices from raw sealed blob and policy digest bytes.
    #[must_use = "dropping it discards the TPM2 token"]
    pub fn new(pcrs: &[u32], blob: &[u8], policy_digest: &[u8]) -> Self {
        Self {
            r#type: String::from("tpm2"),
            keyslots: vec![String::from("0")],
            tpm2_pcrs: pcrs.to_vec(),
            tpm2_hash_alg: String::from("sha256"),
            tpm2_blob: Base64::encode_string(blob),
            tpm2_policy_hash: Base64::encode_string(policy_digest),
        }
    }

    /// Decodes the sealed TPM2 blob bytes stored in the token.
    ///
    /// # Errors
    ///
    /// Returns an error if the stored blob is not valid Base64.
    pub fn sealed_blob(&self) -> Result<Vec<u8>> {
        Base64::decode_vec(&self.tpm2_blob).map_err(Luks2Error::from)
    }

    /// Decodes the TPM2 policy digest stored in the token.
    ///
    /// # Errors
    ///
    /// Returns an error if the stored digest is not valid Base64 or does not
    /// decode to a 32-byte hash.
    pub fn policy_hash(&self) -> Result<[u8; 32]> {
        let mut digest = [0_u8; 32];
        let decoded_len = Base64::decode(&self.tpm2_policy_hash, &mut digest)
            .map_err(Luks2Error::from)?
            .len();
        if decoded_len != digest.len() {
            return Err(Luks2Error::InvalidField(
                "tpm2-policy-hash is not a 32-byte digest".into(),
            ));
        }
        Ok(digest)
    }
}

/// Deserializes every TPM2 token stored in a LUKS2 token map, ordered by token ID.
///
/// # Errors
///
/// Returns an error when a TPM2 token entry does not decode into a token.
pub(crate) fn decode_all(tokens: &HashMap<String, serde_json::Value>) -> Result<Vec<Tpm2Token>> {
    let mut entries = tokens
        .iter()
        .filter(|entry| is_tpm2(entry.1))
        .collect::<Vec<_>>();
    entries.sort_by(|left, right| {
        let left_id = left.0.parse::<u32>().ok();
        let right_id = right.0.parse::<u32>().ok();
        left_id.cmp(&right_id).then_with(|| left.0.cmp(right.0))
    });

    entries
        .into_iter()
        .map(|entry| Tpm2Token::deserialize(entry.1).map_err(Luks2Error::from))
        .collect()
}

/// Returns whether a raw token entry has the TPM2 type discriminator.
pub(crate) fn is_tpm2(value: &serde_json::Value) -> bool {
    value.get("type").and_then(|token_type| token_type.as_str()) == Some("tpm2")
}

#[cfg(test)]
mod tests {
    use base64ct::Encoding;

    use super::*;
    use crate::metadata::Metadata;

    fn token(blob: &str) -> Tpm2Token {
        Tpm2Token {
            r#type: String::from("tpm2"),
            keyslots: vec![String::from("0")],
            tpm2_pcrs: vec![11],
            tpm2_hash_alg: String::from("sha256"),
            tpm2_blob: String::from(blob),
            tpm2_policy_hash: Base64::encode_string(&[0x42_u8; 32]),
        }
    }

    fn foreign_entry() -> serde_json::Value {
        serde_json::json!({"type": "other"})
    }

    #[test]
    fn new_encodes_bytes_and_sealed_blob_round_trips() {
        // ARRANGE
        let blob = [1_u8, 2, 3];
        let policy_digest = [4_u8; 32];

        // ACT
        let token = Tpm2Token::new(&[11], &blob, &policy_digest);

        // ASSERT
        assert_eq!(token.tpm2_blob, <Base64 as Encoding>::encode_string(&blob));
        assert_eq!(
            token.tpm2_policy_hash,
            <Base64 as Encoding>::encode_string(&policy_digest)
        );
        assert_eq!(token.tpm2_pcrs, vec![11]);
        assert_eq!(token.keyslots, vec![String::from("0")]);
        assert_eq!(token.sealed_blob().unwrap(), blob.to_vec());
    }

    #[test]
    fn policy_hash_decodes_the_stored_digest() {
        // ARRANGE
        let digest = [0xAB_u8; 32];
        let token = Tpm2Token::new(&[11], b"blob", &digest);

        // ACT
        let decoded = token.policy_hash().unwrap();

        // ASSERT
        assert_eq!(decoded, digest);
    }

    #[test]
    fn policy_hash_rejects_a_wrong_length() {
        // ARRANGE
        let token = Tpm2Token::new(&[11], b"blob", &[0xAB_u8; 31]);

        // ACT
        let result = token.policy_hash();

        // ASSERT
        assert!(matches!(result, Err(Luks2Error::InvalidField(_))));
    }

    #[test]
    fn policy_hash_rejects_invalid_base64() {
        // ARRANGE
        let mut token = token("blob");
        token.tpm2_policy_hash = String::from("not base64!!");

        // ACT
        let result = token.policy_hash();

        // ASSERT
        assert!(matches!(result, Err(Luks2Error::Base64(_))));
    }

    #[test]
    fn decode_all_orders_tokens_by_numeric_id() {
        // ARRANGE
        let mut tokens = HashMap::new();
        tokens.insert(
            String::from("10"),
            serde_json::to_value(token("ten")).unwrap(),
        );
        tokens.insert(
            String::from("2"),
            serde_json::to_value(token("two")).unwrap(),
        );

        // ACT
        let decoded = decode_all(&tokens).unwrap();

        // ASSERT
        assert_eq!(decoded.len(), 2);
        assert_eq!(decoded[0].tpm2_blob, "two");
        assert_eq!(decoded[1].tpm2_blob, "ten");
    }

    #[test]
    fn decode_all_skips_a_foreign_entry() {
        // ARRANGE
        let mut tokens = HashMap::new();
        tokens.insert(String::from("0"), foreign_entry());
        tokens.insert(
            String::from("1"),
            serde_json::to_value(token("one")).unwrap(),
        );

        // ACT
        let decoded = decode_all(&tokens).unwrap();

        // ASSERT
        assert_eq!(decoded.len(), 1);
        assert_eq!(decoded[0].tpm2_blob, "one");
    }

    #[test]
    fn decode_all_orders_unparsable_ids_by_string() {
        // ARRANGE
        let mut tokens = HashMap::new();
        tokens.insert(
            String::from("beta"),
            serde_json::to_value(token("beta")).unwrap(),
        );
        tokens.insert(
            String::from("alpha"),
            serde_json::to_value(token("alpha")).unwrap(),
        );

        // ACT
        let decoded = decode_all(&tokens).unwrap();

        // ASSERT
        assert_eq!(decoded.len(), 2);
        assert_eq!(decoded[0].tpm2_blob, "alpha");
        assert_eq!(decoded[1].tpm2_blob, "beta");
    }

    #[test]
    fn add_tpm2_token_keeps_existing_tokens() {
        // ARRANGE
        let mut meta = Metadata::new(4096);
        let first = token("first");
        let second = token("second");

        // ACT
        meta.add_tpm2_token(&first).unwrap();
        meta.add_tpm2_token(&second).unwrap();
        let tokens = meta.tpm2_tokens().unwrap();

        // ASSERT
        assert_eq!(tokens.len(), 2);
        assert_eq!(tokens[0].tpm2_blob, "first");
        assert_eq!(tokens[1].tpm2_blob, "second");
        assert_eq!(meta.tokens["0"]["tpm2-blob"], "first");
        assert_eq!(meta.tokens["1"]["tpm2-blob"], "second");
    }

    #[test]
    fn add_tpm2_token_keeps_a_foreign_entry() {
        // ARRANGE
        let mut meta = Metadata::new(4096);
        meta.tokens.insert(String::from("0"), foreign_entry());

        // ACT
        meta.add_tpm2_token(&token("first")).unwrap();
        let tokens = meta.tpm2_tokens().unwrap();

        // ASSERT
        assert_eq!(meta.tokens["0"], foreign_entry());
        assert_eq!(meta.tokens["1"]["tpm2-blob"], "first");
        assert_eq!(tokens.len(), 1);
        assert_eq!(tokens[0].tpm2_blob, "first");
    }

    #[test]
    fn set_tpm2_tokens_replaces_all_tokens() {
        // ARRANGE
        let mut meta = Metadata::new(4096);
        meta.add_tpm2_token(&token("old"))
            .expect("adding the old token should succeed");
        let replacement = token("new");

        // ACT
        meta.set_tpm2_tokens(std::slice::from_ref(&replacement))
            .expect("replacing the token set should succeed");
        let tokens = meta.tpm2_tokens().expect("reading tokens should succeed");

        // ASSERT
        assert_eq!(tokens.len(), 1);
        assert_eq!(tokens[0].tpm2_blob, "new");
        assert!(meta.tokens.contains_key("0"));
    }

    #[test]
    fn set_tpm2_tokens_keeps_a_foreign_entry() {
        // ARRANGE
        let mut meta = Metadata::new(4096);
        meta.tokens.insert(String::from("0"), foreign_entry());

        // ACT
        meta.set_tpm2_tokens(&[token("new")])
            .expect("replacing the token set should succeed");
        let tokens = meta.tpm2_tokens().expect("reading tokens should succeed");

        // ASSERT
        assert_eq!(meta.tokens["0"], foreign_entry());
        assert_eq!(meta.tokens["1"]["tpm2-blob"], "new");
        assert_eq!(tokens.len(), 1);
        assert_eq!(tokens[0].tpm2_blob, "new");
    }
}
