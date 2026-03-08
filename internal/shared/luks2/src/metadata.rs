//! LUKS2 JSON metadata types.
//!
//! Represents the JSON area that follows each binary header on disk.
//! All numeric values stored as decimal strings per the LUKS2 specification.

use std::collections::HashMap;

use base64ct::{Base64, Encoding};
use serde::{Deserialize, Serialize};

use crate::constants::{
    AF_STRIPES, CIPHER_SPEC, DEFAULT_HEADER_SIZE, DEFAULT_JSON_SIZE, DEFAULT_KEYSLOT_AREA_OFFSET,
    DEFAULT_KEYSLOT_AREA_SIZE, DEFAULT_KEYSLOTS_SIZE, VOLUME_KEY_SIZE,
};
use crate::error::{Error, Result};

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

/// Anti-forensic splitter parameters.
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
    pub digest: String,
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
            "0".to_string(),
            Segment {
                r#type: "crypt".to_string(),
                offset: DEFAULT_HEADER_SIZE.to_string(),
                iv_tweak: "0".to_string(),
                size: "dynamic".to_string(),
                encryption: CIPHER_SPEC.to_string(),
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
            r#type: "luks2".to_string(),
            key_size: VOLUME_KEY_SIZE as u32,
            kdf: Kdf {
                // NOTE: Those parameters are fine because the entropy of the key is already 512
                // bits due to random key generation of 64 bytes created during install
                r#type: "argon2id".to_string(),
                salt: Base64::encode_string(kdf_salt),
                time: Some(1),
                memory: Some(65_536),
                cpus: Some(4),
            },
            af: AntiForensic {
                r#type: "luks1".to_string(),
                stripes: AF_STRIPES,
                hash: "sha256".to_string(),
            },
            area: KeyslotArea {
                r#type: "raw".to_string(),
                offset: DEFAULT_KEYSLOT_AREA_OFFSET.to_string(),
                size: DEFAULT_KEYSLOT_AREA_SIZE.to_string(),
                encryption: CIPHER_SPEC.to_string(),
                key_size: VOLUME_KEY_SIZE as u32,
            },
        };

        self.keyslots.insert(id.to_string(), keyslot);
    }

    /// Serializes the metadata to a JSON byte buffer padded to `json_size`.
    pub fn serialize(&self, json_size: u64) -> Result<Vec<u8>> {
        let json = serde_json::to_vec_pretty(self)?;
        if json.len() > json_size as usize {
            return Err(Error::InvalidField(format!(
                "JSON size {} exceeds buffer size {}",
                json.len(),
                json_size
            )));
        }
        let mut buf = vec![0u8; json_size as usize];
        buf[..json.len()].copy_from_slice(&json);
        Ok(buf)
    }

    /// Deserializes metadata from a raw JSON area buffer.
    pub fn deserialize(data: &[u8]) -> Result<Self> {
        let end = data.iter().position(|&b| b == 0).unwrap_or(data.len());
        Ok(serde_json::from_slice(&data[..end])?)
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
            .find(|v| v.get("type").and_then(|t| t.as_str()) == Some("tpm2"))
            .ok_or(Error::NoTpm2Token)?;
        Ok(serde_json::from_value(value.clone())?)
    }

    /// Finds existing TPM2 token ID or allocates the next available one.
    fn find_or_alloc_tpm2_token_id(&self) -> String {
        if let Some(id) = self.tokens.iter().find_map(|(id, v)| {
            (v.get("type").and_then(|t| t.as_str()) == Some("tpm2")).then(|| id.clone())
        }) {
            return id;
        }

        let mut next_id = 0u32;
        while self.tokens.contains_key(&next_id.to_string()) {
            next_id += 1;
        }
        next_id.to_string()
    }
}

#[cfg(test)]
mod tests {
    use base64ct::Encoding;

    use super::*;

    #[test]
    fn test_metadata_new() {
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
    fn test_add_keyslot() {
        // ARRANGE
        let sector_size = 4096;
        let mut meta = Metadata::new(sector_size);
        let kdf_salt = [0x42u8; 64];

        // ACT
        meta.add_keyslot("0", &kdf_salt);

        // ASSERT
        assert_eq!(meta.keyslots.len(), 1);
        let keyslot = meta.keyslots.get("0").unwrap();

        assert_eq!(keyslot.r#type, "luks2");
        assert_eq!(keyslot.key_size, VOLUME_KEY_SIZE as u32);
        assert_eq!(keyslot.kdf.r#type, "argon2id");
        assert_eq!(keyslot.af.stripes, AF_STRIPES);
        assert_eq!(keyslot.af.hash, "sha256");
        assert_eq!(keyslot.area.encryption, CIPHER_SPEC);
        assert_eq!(keyslot.area.offset, DEFAULT_KEYSLOT_AREA_OFFSET.to_string());

        let decoded_salt = base64ct::Base64::decode_vec(&keyslot.kdf.salt).unwrap();
        assert_eq!(decoded_salt, kdf_salt);
    }

    #[test]
    fn test_add_multiple_keyslots() {
        // ARRANGE
        let sector_size = 4096;
        let mut meta = Metadata::new(sector_size);

        let salt1 = [0x01u8; 64];
        let salt2 = [0x02u8; 64];

        // ACT
        meta.add_keyslot("0", &salt1);
        meta.add_keyslot("1", &salt2);

        // ASSERT
        assert_eq!(meta.keyslots.len(), 2);
        assert!(meta.keyslots.contains_key("0"));
        assert!(meta.keyslots.contains_key("1"));
    }

    #[test]
    fn test_serialize_deserialize_roundtrip() {
        // ARRANGE
        let sector_size = 4096;
        let mut meta = Metadata::new(sector_size);
        let kdf_salt = [0x42u8; 64];
        meta.add_keyslot("0", &kdf_salt);

        let json_size = 4096u64;

        // ACT
        let serialized = meta.serialize(json_size).unwrap();

        // ASSERT
        assert_eq!(serialized.len(), json_size as usize);

        // ACT
        let deserialized = Metadata::deserialize(&serialized).unwrap();

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
    fn test_deserialize_with_null_padding() {
        // ARRANGE
        let json = r#"{"keyslots":{},"tokens":{},"segments":{"0":{"type":"crypt","offset":"16777216","iv_tweak":"0","size":"dynamic","encryption":"aes-xts-plain64","sector_size":4096}},"digests":{},"config":{"json_size":"12288","keyslots_size":"16744448"}}"#;
        let mut data = json.as_bytes().to_vec();
        data.extend(vec![0u8; 100]);

        // ACT
        let result = Metadata::deserialize(&data);

        // ASSERT
        assert!(result.is_ok());
    }

    #[test]
    fn test_serialize_padding() {
        // ARRANGE
        let sector_size = 4096;
        let meta = Metadata::new(sector_size);

        let json_size = 4096u64;

        // ACT
        let serialized = meta.serialize(json_size).unwrap();

        // ASSERT
        let mut json_end = serialized.len();
        for i in 0..serialized.len() {
            if serialized[i] == 0 {
                json_end = i;
                break;
            }
        }

        for i in json_end..serialized.len() {
            assert_eq!(serialized[i], 0, "Byte at position {} should be null", i);
        }
    }

    #[test]
    fn test_segment_structure() {
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
    fn test_keyslot_kdf_parameters() {
        // ARRANGE
        let sector_size = 4096;
        let mut meta = Metadata::new(sector_size);
        let kdf_salt = [0x42u8; 64];

        // ACT
        meta.add_keyslot("0", &kdf_salt);
        let keyslot = meta.keyslots.get("0").unwrap();

        // ASSERT
        assert_eq!(keyslot.kdf.time, Some(1));
        assert_eq!(keyslot.kdf.memory, Some(65536));
        assert_eq!(keyslot.kdf.cpus, Some(4));
    }

    #[test]
    fn test_keyslot_area_parameters() {
        // ARRANGE
        let sector_size = 4096;
        let mut meta = Metadata::new(sector_size);
        let kdf_salt = [0x42u8; 64];

        // ACT
        meta.add_keyslot("0", &kdf_salt);
        let keyslot = meta.keyslots.get("0").unwrap();

        // ASSERT
        assert_eq!(keyslot.area.r#type, "raw");
        assert_eq!(keyslot.area.offset, DEFAULT_KEYSLOT_AREA_OFFSET.to_string());
        assert_eq!(keyslot.area.size, DEFAULT_KEYSLOT_AREA_SIZE.to_string());
        assert_eq!(keyslot.area.encryption, CIPHER_SPEC);
        assert_eq!(keyslot.area.key_size, VOLUME_KEY_SIZE as u32);
    }

    #[test]
    fn test_af_structure() {
        // ARRANGE
        let sector_size = 4096;
        let mut meta = Metadata::new(sector_size);
        let kdf_salt = [0x42u8; 64];

        // ACT
        meta.add_keyslot("0", &kdf_salt);
        let keyslot = meta.keyslots.get("0").unwrap();

        // ASSERT
        assert_eq!(keyslot.af.r#type, "luks1");
        assert_eq!(keyslot.af.stripes, AF_STRIPES);
        assert_eq!(keyslot.af.hash, "sha256");
    }

    #[test]
    fn test_deserialize_invalid_json() {
        // ARRANGE
        let data = b"not valid json".to_vec();

        // ACT
        let result = Metadata::deserialize(&data);

        // ASSERT
        assert!(result.is_err());
    }

    #[test]
    fn test_different_sector_sizes() {
        // ACT & ASSERT
        for &sector_size in &[512, 1024, 2048, 4096, 8192] {
            let meta = Metadata::new(sector_size);
            let segment = meta.segments.get("0").unwrap();
            assert_eq!(segment.sector_size, sector_size);
        }
    }

    #[test]
    fn test_serialize_json_size_limits() {
        // ARRANGE
        let sector_size = 4096;
        let meta = Metadata::new(sector_size);

        // ACT
        let small_size = 256u64;
        let result = meta.serialize(small_size);
        let large_size = 4096u64;
        let serialized = meta.serialize(large_size).unwrap();

        // ASSERT
        assert!(result.is_err());
        assert_eq!(serialized.len(), large_size as usize);
    }
}
