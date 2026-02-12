//! LUKS2 JSON metadata types.
//!
//! Represents the JSON area that follows each binary header on disk.
//! All numeric values stored as decimal strings per the LUKS2 specification.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::constants::{
    AF_STRIPES, CIPHER_SPEC, DEFAULT_HEADER_SIZE, DEFAULT_JSON_SIZE, DEFAULT_KEYSLOT_AREA_OFFSET,
    DEFAULT_KEYSLOT_AREA_SIZE, DEFAULT_KEYSLOTS_SIZE, VOLUME_KEY_SIZE,
};

/// Top-level LUKS2 JSON metadata.
#[derive(Debug, Serialize, Deserialize)]
pub struct Metadata {
    pub keyslots: HashMap<String, Keyslot>,
    pub tokens: HashMap<String, serde_json::Value>,
    pub segments: HashMap<String, Segment>,
    pub digests: HashMap<String, Digest>,
    pub config: Config,
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
    ///
    /// The keyslot and digest fields are left empty and must be populated
    /// after key generation and passphrase protection.
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
        use base64ct::{Base64, Encoding};

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
    pub fn serialize(&self, json_size: u64) -> crate::error::Result<Vec<u8>> {
        let json = serde_json::to_vec_pretty(self)?;
        let mut buf = vec![0u8; json_size as usize];
        let copy_len = json.len().min(buf.len());
        buf[..copy_len].copy_from_slice(&json[..copy_len]);
        Ok(buf)
    }

    /// Deserializes metadata from a raw JSON area buffer.
    pub fn deserialize(data: &[u8]) -> crate::error::Result<Self> {
        let end = data.iter().position(|&b| b == 0).unwrap_or(data.len());
        Ok(serde_json::from_slice(&data[..end])?)
    }
}
