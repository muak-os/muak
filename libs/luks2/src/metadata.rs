//! LUKS2 JSON metadata types.
//!
//! Represents the JSON area that follows each binary header on disk.
//! All numeric values stored as decimal strings per the LUKS2 specification.

use std::collections::HashMap;

use base64ct::{Base64, Encoding as _};
use serde::{Deserialize, Serialize};

use crate::error::{Luks2Error, Result};

const AF_STRIPES: u32 = 4000;
const CIPHER_SPEC: &str = "aes-xts-plain64";
const DEFAULT_HEADER_SIZE: u64 = 16 * 1024 * 1024;
const DEFAULT_JSON_SIZE: u64 = 12288;
const DEFAULT_KEYSLOT_AREA_OFFSET: u64 = 32768;
const DEFAULT_KEYSLOT_AREA_SIZE: u64 = 64 * 4000;
const DEFAULT_KEYSLOTS_SIZE: u64 = 16_744_448;
const VOLUME_KEY_SIZE_U32: u32 = 64;

/// Top-level LUKS2 JSON metadata.
#[derive(Debug, Serialize, Deserialize)]
pub struct Metadata {
    pub keyslots: HashMap<String, Keyslot>,
    pub tokens: HashMap<String, serde_json::Value>,
    pub segments: HashMap<String, Segment>,
    pub digests: HashMap<String, Digest>,
    pub config: Config,
}

/// TPM2 token stored in LUKS2 metadata.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Tpm2Token {
    pub r#type: String,
    pub keyslots: Vec<String>,
    #[serde(rename = "tpm2-pcrs")]
    pub tpm2_pcrs: Vec<u32>,
    #[serde(rename = "tpm2-hash-alg")]
    pub tpm2_hash_alg: String,
    #[serde(rename = "tpm2-blob")]
    pub tpm2_blob: String,
    #[serde(rename = "tpm2-policy-hash")]
    pub tpm2_policy_hash: String,
}

/// A LUKS2 keyslot entry.
#[derive(Debug, Serialize, Deserialize)]
pub struct Keyslot {
    pub r#type: String,
    pub key_size: u32,
    pub kdf: Kdf,
    pub af: AntiForensic,
    pub area: KeyslotArea,
}

/// Key derivation function parameters.
#[derive(Debug, Serialize, Deserialize)]
pub struct Kdf {
    pub r#type: String,
    pub salt: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub time: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub memory: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cpus: Option<u32>,
}

/// Antiforensic splitter parameters.
#[derive(Debug, Serialize, Deserialize)]
pub struct AntiForensic {
    pub r#type: String,
    pub stripes: u32,
    pub hash: String,
}

/// Keyslot binary area location and encryption.
#[derive(Debug, Serialize, Deserialize)]
pub struct KeyslotArea {
    pub r#type: String,
    pub offset: String,
    pub size: String,
    pub encryption: String,
    pub key_size: u32,
}

/// An encrypted data segment.
#[derive(Debug, Serialize, Deserialize)]
pub struct Segment {
    pub r#type: String,
    pub offset: String,
    pub iv_tweak: String,
    pub size: String,
    pub encryption: String,
    pub sector_size: u32,
}

/// A volume key digest for verification.
#[derive(Debug, Serialize, Deserialize)]
pub struct Digest {
    pub r#type: String,
    pub keyslots: Vec<String>,
    pub segments: Vec<String>,
    pub hash: String,
    pub iterations: u32,
    pub salt: String,
    #[serde(rename = "digest")]
    pub value: String,
}

/// Header configuration section.
#[derive(Debug, Serialize, Deserialize)]
pub struct Config {
    pub json_size: String,
    pub keyslots_size: String,
}

impl Metadata {
    /// Creates default metadata for a new LUKS2 volume.
    pub fn new(sector_size: u32) -> Self {
        let mut segments = HashMap::new();
        segments.insert(
            String::from("0"),
            Segment {
                r#type: String::from("crypt"),
                offset: DEFAULT_HEADER_SIZE.to_string(),
                iv_tweak: String::from("0"),
                size: String::from("dynamic"),
                encryption: CIPHER_SPEC.to_owned(),
                sector_size,
            },
        );

        Self {
            keyslots: HashMap::new(),
            tokens: HashMap::new(),
            segments,
            digests: HashMap::new(),
            config: Config {
                json_size: DEFAULT_JSON_SIZE.to_string(),
                keyslots_size: DEFAULT_KEYSLOTS_SIZE.to_string(),
            },
        }
    }

    /// Adds a keyslot entry with the given Argon2id parameters and salt.
    pub fn add_keyslot(&mut self, id: &str, kdf_salt: &[u8]) {
        let keyslot = Keyslot {
            r#type: String::from("luks2"),
            key_size: VOLUME_KEY_SIZE_U32,
            kdf: Kdf {
                // NOTE: Those parameters are fine because the entropy of the key is already 512
                // bits due to random key generation of 64 bytes created during install
                r#type: String::from("argon2id"),
                salt: Base64::encode_string(kdf_salt),
                time: Some(1),
                memory: Some(65_536),
                cpus: Some(4),
            },
            af: AntiForensic {
                r#type: String::from("luks1"),
                stripes: AF_STRIPES,
                hash: String::from("sha256"),
            },
            area: KeyslotArea {
                r#type: String::from("raw"),
                offset: DEFAULT_KEYSLOT_AREA_OFFSET.to_string(),
                size: DEFAULT_KEYSLOT_AREA_SIZE.to_string(),
                encryption: CIPHER_SPEC.to_owned(),
                key_size: VOLUME_KEY_SIZE_U32,
            },
        };

        self.keyslots.insert(id.to_owned(), keyslot);
    }

    /// Serializes the metadata to a JSON byte buffer padded to `json_size`.
    pub fn to_json_buffer(&self, json_size: u64) -> Result<Vec<u8>> {
        let json = serde_json::to_vec_pretty(self)?;
        let json_size = usize::try_from(json_size)
            .map_err(|_error| Luks2Error::InvalidField("json size exceeds usize".into()))?;
        if json.len() > json_size {
            return Err(Luks2Error::InvalidField(format!(
                "JSON size {} exceeds buffer size {}",
                json.len(),
                json_size
            )));
        }
        let mut buf = vec![0_u8; json_size];
        let prefix = buf
            .get_mut(..json.len())
            .ok_or_else(|| Luks2Error::InvalidField("json buffer too small".into()))?;
        prefix.copy_from_slice(&json);
        Ok(buf)
    }

    /// Deserializes metadata from a raw JSON area buffer.
    pub fn from_json_buffer(data: &[u8]) -> Result<Self> {
        let end = data
            .iter()
            .position(|&byte| byte == 0)
            .unwrap_or(data.len());
        let json = data
            .get(..end)
            .ok_or_else(|| Luks2Error::InvalidField("json buffer end out of bounds".into()))?;
        Ok(serde_json::from_slice(json)?)
    }

    /// Sets a TPM2 token in the metadata, replacing any existing one.
    pub fn set_tpm2_token(&mut self, token: &Tpm2Token) -> Result<()> {
        let value = serde_json::to_value(token)?;
        let id = self.find_or_alloc_tpm2_token_id();
        self.tokens.insert(id, value);
        Ok(())
    }

    /// Reads the first TPM2 token from metadata.
    pub fn get_tpm2_token(&self) -> Result<Tpm2Token> {
        let value = self
            .tokens
            .values()
            .find(|value| {
                value.get("type").and_then(|token_type| token_type.as_str()) == Some("tpm2")
            })
            .ok_or(Luks2Error::NoTpm2Token)?;
        Ok(serde_json::from_value(value.clone())?)
    }

    /// Finds existing TPM2 token ID or allocates the next available one.
    fn find_or_alloc_tpm2_token_id(&self) -> String {
        if let Some(id) = self.tokens.iter().find_map(|(id, value)| {
            (value.get("type").and_then(|token_type| token_type.as_str()) == Some("tpm2"))
                .then(|| id.clone())
        }) {
            return id;
        }

        let mut next_id = 0_u32;
        while self.tokens.contains_key(&next_id.to_string()) {
            next_id = next_id.saturating_add(1);
        }
        next_id.to_string()
    }
}

#[cfg(test)]
mod tests {
    use base64ct::Encoding as _;

    use super::*;

    #[test]
    fn metadata_new() {
        // ACT
        let sector_size = 4096;
        let meta = Metadata::new(sector_size);

        // ASSERT
        assert!(meta.keyslots.is_empty());
        assert!(meta.tokens.is_empty());
        assert!(meta.digests.is_empty());

        assert_eq!(meta.segments.len(), 1);
        let segment = meta.segments.get("0").unwrap();
        assert_eq!(segment.offset, DEFAULT_HEADER_SIZE.to_string());
        assert_eq!(segment.r#type, "crypt");
        assert_eq!(segment.encryption, CIPHER_SPEC);
        assert_eq!(segment.sector_size, sector_size);

        assert_eq!(meta.config.json_size, DEFAULT_JSON_SIZE.to_string());
        assert_eq!(meta.config.keyslots_size, DEFAULT_KEYSLOTS_SIZE.to_string());
    }

    #[test]
    fn add_keyslot() {
        // ARRANGE
        let sector_size = 4096;
        let mut meta = Metadata::new(sector_size);
        let kdf_salt = [0x42_u8; 64];

        // ACT
        meta.add_keyslot("0", &kdf_salt);

        // ASSERT
        assert_eq!(meta.keyslots.len(), 1);
        let keyslot = meta.keyslots.get("0").unwrap();

        assert_eq!(keyslot.r#type, "luks2");
        assert_eq!(keyslot.key_size, VOLUME_KEY_SIZE_U32);
        assert_eq!(keyslot.kdf.r#type, "argon2id");
        assert_eq!(keyslot.af.stripes, AF_STRIPES);
        assert_eq!(keyslot.af.hash, "sha256");
        assert_eq!(keyslot.area.encryption, CIPHER_SPEC);
        assert_eq!(keyslot.area.offset, DEFAULT_KEYSLOT_AREA_OFFSET.to_string());

        let decoded_salt = base64ct::Base64::decode_vec(&keyslot.kdf.salt).unwrap();
        assert_eq!(decoded_salt, kdf_salt);
    }

    #[test]
    fn add_multiple_keyslots() {
        // ARRANGE
        let sector_size = 4096;
        let mut meta = Metadata::new(sector_size);

        let salt1 = [0x01_u8; 64];
        let salt2 = [0x02_u8; 64];

        // ACT
        meta.add_keyslot("0", &salt1);
        meta.add_keyslot("1", &salt2);

        // ASSERT
        assert_eq!(meta.keyslots.len(), 2);
        assert!(meta.keyslots.contains_key("0"));
        assert!(meta.keyslots.contains_key("1"));
    }

    #[test]
    fn serialize_deserialize_roundtrip() {
        // ARRANGE
        let sector_size = 4096;
        let mut meta = Metadata::new(sector_size);
        let kdf_salt = [0x42_u8; 64];
        meta.add_keyslot("0", &kdf_salt);

        let json_size = 4096_u64;

        // ACT
        let serialized = meta.to_json_buffer(json_size).unwrap();

        // ASSERT
        assert_eq!(serialized.len(), usize::try_from(json_size).unwrap());

        // ACT
        let deserialized = Metadata::from_json_buffer(&serialized).unwrap();

        // ASSERT
        assert_eq!(deserialized.keyslots.len(), meta.keyslots.len());
        assert_eq!(deserialized.segments.len(), meta.segments.len());
        assert_eq!(deserialized.config.json_size, meta.config.json_size);

        let original_keyslot = meta.keyslots.get("0").unwrap();
        let deserialized_keyslot = deserialized.keyslots.get("0").unwrap();
        assert_eq!(deserialized_keyslot.key_size, original_keyslot.key_size);
        assert_eq!(deserialized_keyslot.kdf.r#type, original_keyslot.kdf.r#type);
    }

    #[test]
    fn deserialize_with_null_padding() {
        // ARRANGE
        let json = r#"{"keyslots":{},"tokens":{},"segments":{"0":{"type":"crypt","offset":"16777216","iv_tweak":"0","size":"dynamic","encryption":"aes-xts-plain64","sector_size":4096}},"digests":{},"config":{"json_size":"12288","keyslots_size":"16744448"}}"#;
        let mut data = json.as_bytes().to_vec();
        data.extend(vec![0_u8; 100]);

        // ACT
        let result = Metadata::from_json_buffer(&data);

        // ASSERT
        result.unwrap();
    }

    #[test]
    fn serialize_padding() {
        // ARRANGE
        let sector_size = 4096;
        let meta = Metadata::new(sector_size);

        let json_size = 4096_u64;

        // ACT
        let serialized = meta.to_json_buffer(json_size).unwrap();

        // ASSERT
        let json_end = serialized
            .iter()
            .position(|byte| *byte == 0)
            .unwrap_or(serialized.len());

        for (index, byte) in serialized.iter().enumerate().skip(json_end) {
            assert_eq!(*byte, 0, "Byte at position {index} should be null");
        }
    }

    #[test]
    fn segment_structure() {
        // ARRANGE
        let sector_size = 4096;
        let meta = Metadata::new(sector_size);

        // ACT
        let segment = meta.segments.get("0").unwrap();

        // ASSERT
        assert_eq!(segment.r#type, "crypt");
        assert_eq!(segment.offset, DEFAULT_HEADER_SIZE.to_string());
        assert_eq!(segment.iv_tweak, "0");
        assert_eq!(segment.size, "dynamic");
        assert_eq!(segment.encryption, CIPHER_SPEC);
        assert_eq!(segment.sector_size, sector_size);
    }

    #[test]
    fn keyslot_kdf_parameters() {
        // ARRANGE
        let sector_size = 4096;
        let mut meta = Metadata::new(sector_size);
        let kdf_salt = [0x42_u8; 64];

        // ACT
        meta.add_keyslot("0", &kdf_salt);
        let keyslot = meta.keyslots.get("0").unwrap();

        // ASSERT
        assert_eq!(keyslot.kdf.time, Some(1));
        assert_eq!(keyslot.kdf.memory, Some(65536));
        assert_eq!(keyslot.kdf.cpus, Some(4));
    }

    #[test]
    fn keyslot_area_parameters() {
        // ARRANGE
        let sector_size = 4096;
        let mut meta = Metadata::new(sector_size);
        let kdf_salt = [0x42_u8; 64];

        // ACT
        meta.add_keyslot("0", &kdf_salt);
        let keyslot = meta.keyslots.get("0").unwrap();

        // ASSERT
        assert_eq!(keyslot.area.r#type, "raw");
        assert_eq!(keyslot.area.offset, DEFAULT_KEYSLOT_AREA_OFFSET.to_string());
        assert_eq!(keyslot.area.size, DEFAULT_KEYSLOT_AREA_SIZE.to_string());
        assert_eq!(keyslot.area.encryption, CIPHER_SPEC);
        assert_eq!(keyslot.area.key_size, VOLUME_KEY_SIZE_U32);
    }

    #[test]
    fn af_structure() {
        // ARRANGE
        let sector_size = 4096;
        let mut meta = Metadata::new(sector_size);
        let kdf_salt = [0x42_u8; 64];

        // ACT
        meta.add_keyslot("0", &kdf_salt);
        let keyslot = meta.keyslots.get("0").unwrap();

        // ASSERT
        assert_eq!(keyslot.af.r#type, "luks1");
        assert_eq!(keyslot.af.stripes, AF_STRIPES);
        assert_eq!(keyslot.af.hash, "sha256");
    }

    #[test]
    fn deserialize_invalid_json() {
        // ARRANGE
        let data = b"not valid json".to_vec();

        // ACT
        let result = Metadata::from_json_buffer(&data);

        // ASSERT
        result.unwrap_err();
    }

    #[test]
    fn different_sector_sizes() {
        // ACT & ASSERT
        for &sector_size in &[512, 1024, 2048, 4096, 8192] {
            let meta = Metadata::new(sector_size);
            let segment = meta.segments.get("0").unwrap();
            assert_eq!(segment.sector_size, sector_size);
        }
    }

    #[test]
    fn serialize_json_size_limits() {
        // ARRANGE
        let sector_size = 4096;
        let meta = Metadata::new(sector_size);

        // ACT
        let small_size = 256_u64;
        let result = meta.to_json_buffer(small_size);
        let large_size = 4096_u64;
        let serialized = meta.to_json_buffer(large_size).unwrap();

        // ASSERT
        result.unwrap_err();
        assert_eq!(serialized.len(), usize::try_from(large_size).unwrap());
    }

    #[test]
    fn set_tpm2_token_reuses_existing_tpm2_id() {
        // ARRANGE
        let mut meta = Metadata::new(4096);
        meta.tokens.insert(
            String::from("7"),
            serde_json::json!({ "type": "tpm2", "tpm2-blob": "old" }),
        );
        let token = Tpm2Token {
            r#type: String::from("tpm2"),
            keyslots: vec![String::from("0")],
            tpm2_pcrs: vec![11],
            tpm2_hash_alg: String::from("sha256"),
            tpm2_blob: String::from("new"),
            tpm2_policy_hash: String::from("policy"),
        };

        // ACT
        meta.set_tpm2_token(&token).unwrap();

        // ASSERT
        assert_eq!(meta.tokens.len(), 1);
        assert_eq!(
            meta.tokens.get("7").unwrap().get("tpm2-blob").unwrap(),
            "new"
        );
    }

    #[test]
    fn set_tpm2_token_allocates_next_available_id() {
        // ARRANGE
        let mut meta = Metadata::new(4096);
        meta.tokens
            .insert(String::from("0"), serde_json::json!({ "type": "other" }));
        meta.tokens
            .insert(String::from("1"), serde_json::json!({ "type": "other" }));
        let token = Tpm2Token {
            r#type: String::from("tpm2"),
            keyslots: vec![String::from("0")],
            tpm2_pcrs: vec![11],
            tpm2_hash_alg: String::from("sha256"),
            tpm2_blob: String::from("blob"),
            tpm2_policy_hash: String::from("policy"),
        };

        // ACT
        meta.set_tpm2_token(&token).unwrap();

        // ASSERT
        assert!(meta.tokens.contains_key("2"));
    }

    #[test]
    fn get_tpm2_token_ignores_non_tpm2_entries() {
        // ARRANGE
        let mut meta = Metadata::new(4096);
        meta.tokens
            .insert(String::from("0"), serde_json::json!({ "type": "other" }));
        meta.tokens.insert(
            String::from("1"),
            serde_json::json!({
                "type": "tpm2",
                "keyslots": ["0"],
                "tpm2-pcrs": [11],
                "tpm2-hash-alg": "sha256",
                "tpm2-blob": "blob",
                "tpm2-policy-hash": "policy"
            }),
        );

        // ACT
        let token = meta.get_tpm2_token().unwrap();

        // ASSERT
        assert_eq!(token.r#type, "tpm2");
        assert_eq!(token.tpm2_blob, "blob");
    }
}
