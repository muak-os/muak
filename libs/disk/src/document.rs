//! The serialized disk document (`disk.toml`) authored at build time.

use serde::{Deserialize, Serialize};

use crate::error::DiskError;
use crate::plan::Size;
use crate::role::Role;

/// Schema version of the disk document.
pub const API_VERSION: &str = "muak.dev/disk/v1-beta";

/// One planned partition, as recorded in the document.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Partition {
    /// Functional role used by consumers for lookup.
    pub role: Role,
    /// GPT partition name.
    pub name: String,
    /// GPT partition type GUID.
    pub type_guid: String,
    /// Partition size.
    pub size: Size,
}

/// The serialized disk document authored at build time and applied by the installer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Document {
    /// Schema version of this document.
    api_version: String,
    /// Whether the install wipes the disk before placing the partitions.
    wipe: bool,
    /// All Muak-managed partitions of the install, in placement order.
    partitions: Vec<Partition>,
}

impl Document {
    /// Creates a plan document for the given partitions.
    #[must_use = "dropping it discards the parsed disk document"]
    pub fn new(wipe: bool, partitions: Vec<Partition>) -> Self {
        Self {
            api_version: API_VERSION.to_owned(),
            wipe,
            partitions,
        }
    }

    /// Returns the first partition with the given role.
    #[must_use = "callers must handle a role the document does not declare"]
    pub fn find(&self, role: Role) -> Option<&Partition> {
        self.partitions
            .iter()
            .find(|partition| partition.role == role)
    }

    /// Returns whether the install wipes the disk before placing partitions.
    #[must_use = "callers must branch on whether the disk is wiped"]
    pub const fn wipe(&self) -> bool {
        self.wipe
    }

    /// Returns all planned partitions, in placement order.
    #[must_use = "callers need the partitions to plan the layout"]
    pub fn partitions(&self) -> &[Partition] {
        &self.partitions
    }

    /// Deserializes and validates a plan document from TOML.
    ///
    /// # Errors
    ///
    /// Returns [`DiskError`] when parsing fails or the document uses an unknown schema version.
    pub fn from_toml(bytes: &str) -> Result<Self, DiskError> {
        let doc: Self = toml::from_str(bytes).map_err(|e| DiskError::Toml(e.to_string()))?;

        if doc.api_version != API_VERSION {
            return Err(DiskError::UnsupportedApiVersion(doc.api_version));
        }

        Ok(doc)
    }

    /// Serializes the plan document to TOML.
    ///
    /// # Errors
    ///
    /// Returns [`DiskError`] when serialization fails.
    pub fn to_toml(&self) -> Result<String, DiskError> {
        toml::to_string(self).map_err(|e| DiskError::Toml(e.to_string()))
    }

    /// Reads and validates a plan document from `path`.
    ///
    /// # Errors
    ///
    /// Returns [`DiskError`] when reading or validation fails.
    pub fn read(path: &std::path::Path) -> Result<Self, DiskError> {
        Self::from_toml(&std::fs::read_to_string(path)?)
    }
}

/// Formats raw GPT GUID bytes as a hyphenated UUID string.
#[must_use = "callers need the formatted GUID for the partition table"]
pub fn guid(bytes: &[u8; 16]) -> String {
    uuid::Uuid::from_bytes(*bytes).hyphenated().to_string()
}
