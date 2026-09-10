//! The persisted disk document describing the installed partitions.

use std::path::Path;

use parttable::gpt::partition::Partition as GptPartition;
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::plan::{Plan, Size};
use crate::role::Role;

/// Schema version of the disk document.
pub const API_VERSION: &str = "muak-disk-v1";

/// Errors produced when reading, writing, or converting disk documents.
#[derive(Debug, Error)]
pub enum DiskError {
    /// The document uses a schema version this reader does not know.
    #[error("unsupported disk document api_version '{0}' (supported: {API_VERSION})")]
    UnsupportedApiVersion(String),
    /// The document could not be parsed or serialized.
    #[error("invalid disk document: {0}")]
    Toml(String),
    /// The document could not be converted to or from a plan.
    #[error("invalid plan conversion: {0}")]
    Plan(String),
    /// The document could not be read from or written to disk.
    #[error("disk document io error")]
    Io(#[from] std::io::Error),
}

/// One recorded partition of the installed system.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Partition {
    /// Functional role used by consumers for lookup.
    pub role: Role,
    /// GPT partition name, discovered at runtime via PARTNAME uevents.
    pub name: String,
    /// GPT partition type GUID.
    pub type_guid: String,
    /// Partition size.
    pub size: Size,
    /// Unique GPT partition GUID, absent from plan documents.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub partuuid: Option<String>,
}

/// The plan authored at build time and the record of what the installer created.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Doc {
    /// Schema version of this document.
    api_version: String,
    /// Whether the install wipes the disk before placing the partitions.
    wipe: bool,
    /// All Muak-managed partitions of the install, in placement order.
    partitions: Vec<Partition>,
}

impl Doc {
    /// Creates a disk document for the given partitions.
    #[must_use]
    pub fn new(wipe: bool, partitions: Vec<Partition>) -> Self {
        Self {
            api_version: API_VERSION.to_owned(),
            wipe,
            partitions,
        }
    }

    /// Returns the first partition with the given role.
    #[must_use]
    pub fn find(&self, role: Role) -> Option<&Partition> {
        self.partitions
            .iter()
            .find(|partition| partition.role == role)
    }

    /// Returns whether the install wipes the disk before placing partitions.
    #[must_use]
    pub const fn wipe(&self) -> bool {
        self.wipe
    }

    /// Returns all recorded partitions, in placement order.
    #[must_use]
    pub fn partitions(&self) -> &[Partition] {
        &self.partitions
    }

    /// Deserializes and validates a disk document from TOML.
    ///
    /// # Errors
    ///
    /// Returns [`DiskError`] when parsing fails or the document uses an
    /// unknown schema version.
    pub fn from_toml(bytes: &str) -> Result<Self, DiskError> {
        let doc: Self = toml::from_str(bytes).map_err(|e| DiskError::Toml(e.to_string()))?;

        if doc.api_version != API_VERSION {
            return Err(DiskError::UnsupportedApiVersion(doc.api_version));
        }

        Ok(doc)
    }

    /// Serializes the disk document to TOML.
    ///
    /// # Errors
    ///
    /// Returns [`DiskError`] when serialization fails.
    pub fn to_toml(&self) -> Result<String, DiskError> {
        toml::to_string(self).map_err(|e| DiskError::Toml(e.to_string()))
    }

    /// Writes the disk document to `path`.
    ///
    /// # Errors
    ///
    /// Returns [`DiskError`] when serialization or writing fails.
    pub fn write(&self, path: &Path) -> Result<(), DiskError> {
        std::fs::write(path, self.to_toml()?)?;
        Ok(())
    }

    /// Reads and validates a disk document from `path`.
    ///
    /// # Errors
    ///
    /// Returns [`DiskError`] when reading or validation fails.
    pub fn read(path: &Path) -> Result<Self, DiskError> {
        Self::from_toml(&std::fs::read_to_string(path)?)
    }

    /// Builds the document from a partition plan (no recorded partuuids).
    ///
    /// # Errors
    ///
    /// Returns [`DiskError`] when a plan partition carries no role.
    pub fn from_plan(plan: &Plan) -> Result<Self, DiskError> {
        let partitions = plan
            .partitions
            .iter()
            .map(partition_from_spec)
            .collect::<Result<Vec<_>, DiskError>>()?;

        Ok(Self {
            api_version: API_VERSION.to_owned(),
            wipe: plan.wipe,
            partitions,
        })
    }

    /// Converts the document back into a partition plan for the executor.
    ///
    /// # Errors
    ///
    /// Returns [`DiskError`] when a type GUID is not a valid UUID.
    pub fn to_plan(&self) -> Result<Plan, DiskError> {
        let partitions = self
            .partitions
            .iter()
            .map(|partition| {
                let type_guid = parse_type_guid(&partition.type_guid)?;

                Ok(crate::plan::PartitionSpec {
                    role: Some(partition.role),
                    name: partition.name.clone(),
                    type_guid,
                    size: partition.size,
                })
            })
            .collect::<Result<Vec<_>, DiskError>>()?;

        Ok(Plan {
            wipe: self.wipe,
            partitions,
        })
    }
}

fn partition_from_spec(spec: &crate::plan::PartitionSpec) -> Result<Partition, DiskError> {
    let role = spec
        .role
        .ok_or_else(|| DiskError::Plan(format!("partition '{}' has no role", spec.name)))?;

    Ok(Partition {
        role,
        name: spec.name.clone(),
        type_guid: guid(&spec.type_guid),
        size: spec.size,
        partuuid: None,
    })
}

/// Resolves the GPT partition name for a role.
#[must_use]
pub fn partition_name(doc: Option<&Doc>, role: Role) -> String {
    doc.and_then(|doc| doc.find(role)).map_or_else(
        || role.gpt_name().to_owned(),
        |partition| partition.name.clone(),
    )
}

fn parse_type_guid(text: &str) -> Result<[u8; 16], DiskError> {
    let parsed = uuid::Uuid::parse_str(text)
        .map_err(|e| DiskError::Plan(format!("bad type GUID '{text}': {e}")))?;

    Ok(parsed.into_bytes())
}

/// Formats raw GPT GUID bytes as a hyphenated UUID string.
#[must_use]
pub fn guid(bytes: &[u8; 16]) -> String {
    uuid::Uuid::from_bytes(*bytes).hyphenated().to_string()
}

/// Builds a document record from a placed GPT partition entry.
#[must_use]
pub fn partition_of(role: Role, partition: &GptPartition, sector_size: u64) -> Partition {
    let size_bytes = partition
        .ending_lba
        .checked_sub(partition.starting_lba)
        .and_then(|spans| spans.checked_add(1))
        .and_then(|lbas| lbas.checked_mul(sector_size))
        .unwrap_or(0);

    Partition {
        role,
        name: partition.name.clone(),
        type_guid: guid(&partition.type_guid),
        size: Size::Fixed(size_bytes),
        partuuid: Some(guid(&partition.unique_guid)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::plan;
    use crate::role::Role;

    fn sample() -> Doc {
        Doc::new(
            true,
            vec![
                Partition {
                    role: Role::Esp,
                    name: "EFI".to_owned(),
                    type_guid: guid(&plan::ESP_TYPE_GUID),
                    size: Size::Fixed(plan::EFI_SIZE),
                    partuuid: Some(guid(&[0xAB; 16])),
                },
                Partition {
                    role: Role::State,
                    name: "STATE".to_owned(),
                    type_guid: guid(&[0x0F; 16]),
                    size: Size::Fixed(plan::STATE_SIZE),
                    partuuid: Some(guid(&[0xCD; 16])),
                },
                Partition {
                    role: Role::Data,
                    name: "DATA".to_owned(),
                    type_guid: guid(&[0x0F; 16]),
                    size: Size::Fill,
                    partuuid: Some(guid(&[0xEF; 16])),
                },
            ],
        )
    }

    #[test]
    fn toml_round_trip_preserves_partitions() {
        // ARRANGE
        let doc = sample();

        // ACT
        let serialized = doc.to_toml().expect("serialize");
        let parsed = Doc::from_toml(&serialized).expect("deserialize");

        // ASSERT
        assert_eq!(parsed, doc, "round trip must preserve the document");
    }

    #[test]
    fn serialized_document_carries_api_version() {
        // ARRANGE
        let doc = sample();

        // ACT
        let serialized = doc.to_toml().expect("serialize");

        // ASSERT
        assert!(
            serialized.contains("api_version = \"muak-disk-v1\""),
            "serialized document must carry the api version: {serialized}"
        );
    }

    #[test]
    fn from_toml_rejects_unknown_api_version() {
        // ARRANGE
        let bytes = "api_version = \"muak-disk-v999\"\nwipe = true\n[[partitions]]\nrole = \"esp\"\nname = \"EFI\"\ntype_guid = \"00000000-0000-0000-0000-000000000000\"\nsize = \"fill\"\n";

        // ACT
        let result = Doc::from_toml(bytes);

        // ASSERT
        assert!(
            matches!(result, Err(DiskError::UnsupportedApiVersion(_))),
            "unknown api_version must be rejected"
        );
    }

    #[test]
    fn find_returns_partition_by_role() {
        // ARRANGE
        let doc = sample();

        // ACT
        let found = doc.find(Role::State).expect("state partition");

        // ASSERT
        assert_eq!(found.name, "STATE", "role lookup must return STATE");
    }

    #[test]
    fn find_returns_none_for_missing_role() {
        // ARRANGE
        let doc = Doc::new(
            true,
            vec![Partition {
                role: Role::Esp,
                name: "EFI".to_owned(),
                type_guid: guid(&plan::ESP_TYPE_GUID),
                size: Size::Fill,
                partuuid: None,
            }],
        );

        // ACT
        let found = doc.find(Role::Data);

        // ASSERT
        assert!(found.is_none(), "absent roles must return none");
    }

    #[test]
    fn write_and_read_round_trip_through_the_filesystem() {
        // ARRANGE
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("disk.toml");
        let doc = sample();

        // ACT
        doc.write(&path).expect("write");
        let loaded = Doc::read(&path).expect("read");

        // ASSERT
        assert_eq!(
            loaded, doc,
            "filesystem round trip must preserve the document"
        );
    }

    #[test]
    fn partition_name_prefers_recorded_name() {
        // ARRANGE
        let doc = Doc::new(
            true,
            vec![Partition {
                role: Role::State,
                name: "SYSTEMVOL".to_owned(),
                type_guid: guid(&[0x0F; 16]),
                size: Size::Fill,
                partuuid: None,
            }],
        );

        // ACT
        let name = partition_name(Some(&doc), Role::State);

        // ASSERT
        assert_eq!(name, "SYSTEMVOL", "recorded names must win over canonical");
    }

    #[test]
    fn partition_name_falls_back_to_canonical_gpt_name() {
        // ARRANGE / ACT
        let name = partition_name(None, Role::State);

        // ASSERT
        assert_eq!(name, "STATE", "absent docs must fall back to the GPT name");
    }

    #[test]
    fn plan_conversion_round_trips_without_partuuids() {
        // ARRANGE
        let (system, _) = plan::uefi(true);

        // ACT
        let doc = Doc::from_plan(&system).expect("from plan");
        let converted = doc.to_plan().expect("to plan");

        // ASSERT
        assert_eq!(converted, system, "plan conversion must round trip");
        assert!(
            doc.partitions
                .iter()
                .all(|partition| partition.partuuid.is_none()),
            "plan documents carry no recorded partuuids"
        );
    }

    #[test]
    fn plan_conversion_rejects_bad_type_guids() {
        // ARRANGE
        let doc = Doc::new(
            true,
            vec![Partition {
                role: Role::Esp,
                name: "EFI".to_owned(),
                type_guid: "not-a-uuid".to_owned(),
                size: Size::Fill,
                partuuid: None,
            }],
        );

        // ACT
        let result = doc.to_plan();

        // ASSERT
        assert!(result.is_err(), "invalid type GUIDs must be rejected");
    }

    #[test]
    fn partition_of_formats_guids_and_computes_size() {
        // ARRANGE
        let gpt_partition = GptPartition {
            type_guid: plan::ESP_TYPE_GUID,
            unique_guid: [0xAB; 16],
            starting_lba: 2048,
            ending_lba: 2048 + 1024 - 1,
            attributes: 0,
            name: "EFI".to_owned(),
        };

        // ACT
        let partition = partition_of(Role::Esp, &gpt_partition, 512);

        // ASSERT
        assert_eq!(partition.name, "EFI");
        assert_eq!(partition.type_guid, guid(&plan::ESP_TYPE_GUID));
        assert_eq!(partition.partuuid, Some(guid(&[0xAB; 16])));
        assert_eq!(partition.size, Size::Fixed(1024 * 512));
    }
}
