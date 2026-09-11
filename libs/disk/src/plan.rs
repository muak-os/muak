//! The partition plan: executor types and their serialized document.

use serde::{Deserialize, Serialize};

use crate::error::DiskError;
use crate::role::Role;

/// GPT type GUID of the EFI System Partition (wire byte order).
pub const ESP_TYPE_GUID: [u8; 16] = [
    0x28, 0x73, 0x2a, 0xc1, 0x1f, 0xf8, 0xd2, 0x11, 0xba, 0x4b, 0x00, 0xa0, 0xc9, 0x3e, 0xc9, 0x3b,
];

/// Frozen size of the EFI System Partition.
pub const EFI_SIZE: u64 = 512 * 1024 * 1024;

/// Frozen size of the STATE partition.
pub const STATE_SIZE: u64 = 1024 * 1024 * 1024;

/// Schema version of the plan document.
pub const API_VERSION: &str = "muak.dev/diskplan/v1-beta";

/// Size of a planned partition.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Size {
    /// Exact size in bytes.
    Fixed(u64),
    /// All remaining usable space on the disk.
    Fill,
}

/// One planned partition.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PartitionSpec {
    /// Functional role; `None` for foreign partitions kept by the platform.
    pub role: Option<Role>,
    /// GPT partition name.
    pub name: String,
    /// GPT partition type GUID.
    pub type_guid: [u8; 16],
    /// Partition size.
    pub size: Size,
}

impl PartitionSpec {
    /// Creates a partition spec for partitions we manage.
    #[must_use]
    pub fn create(role: Role, name: &str, type_guid: [u8; 16], size: Size) -> Self {
        Self {
            role: Some(role),
            name: name.to_owned(),
            type_guid,
            size,
        }
    }
}

/// The partition plan applied to one disk.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Plan {
    /// Whether the disk is wiped before applying the plan.
    pub wipe: bool,
    /// Planned partitions, in placement order.
    pub partitions: Vec<PartitionSpec>,
}

impl Plan {
    /// Creates a plan that wipes the disk before placing the partitions.
    #[must_use]
    pub fn wiped(partitions: Vec<PartitionSpec>) -> Self {
        Self {
            wipe: true,
            partitions,
        }
    }
}

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

/// The serialized plan document (`diskplan.toml`) authored at build time and
/// applied by the installer.
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

    /// Returns all planned partitions, in placement order.
    #[must_use]
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

    /// Writes the plan document to `path`.
    ///
    /// # Errors
    ///
    /// Returns [`DiskError`] when serialization or writing fails.
    pub fn write(&self, path: &std::path::Path) -> Result<(), DiskError> {
        std::fs::write(path, self.to_toml()?)?;
        Ok(())
    }

    /// Reads and validates a plan document from `path`.
    ///
    /// # Errors
    ///
    /// Returns [`DiskError`] when reading or validation fails.
    pub fn read(path: &std::path::Path) -> Result<Self, DiskError> {
        Self::from_toml(&std::fs::read_to_string(path)?)
    }

    /// Builds the document from a partition plan.
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

        Ok(Self::new(plan.wipe, partitions))
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

                Ok(PartitionSpec {
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

fn partition_from_spec(spec: &PartitionSpec) -> Result<Partition, DiskError> {
    let role = spec
        .role
        .ok_or_else(|| DiskError::Plan(format!("partition '{}' has no role", spec.name)))?;

    Ok(Partition {
        role,
        name: spec.name.clone(),
        type_guid: guid(&spec.type_guid),
        size: spec.size,
    })
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

#[cfg(test)]
mod tests {
    use parttable::gpt::partition::LINUX_FS_GUID;

    use super::*;
    use crate::layout::Layout;

    fn sample() -> Document {
        Document::from_plan(&Layout::Uefi.plan()).expect("doc from plan")
    }

    #[test]
    fn create_spec_sets_role_name_and_size() {
        // ARRANGE / ACT
        let spec = PartitionSpec::create(Role::Esp, "EFI", ESP_TYPE_GUID, Size::Fixed(512));

        // ASSERT
        assert_eq!(spec.role, Some(Role::Esp), "role must be set");
        assert_eq!(spec.name, "EFI", "name must be kept");
        assert_eq!(spec.type_guid, ESP_TYPE_GUID, "type GUID must be kept");
        assert_eq!(spec.size, Size::Fixed(512), "size must be kept");
    }

    #[test]
    fn wiped_plan_sets_wipe_flag_and_keeps_order() {
        // ARRANGE
        let specs = vec![
            PartitionSpec::create(Role::Esp, "EFI", ESP_TYPE_GUID, Size::Fixed(512)),
            PartitionSpec::create(Role::State, "STATE", LINUX_FS_GUID, Size::Fill),
        ];

        // ACT
        let plan = Plan::wiped(specs.clone());

        // ASSERT
        assert!(plan.wipe, "wiped() must set the wipe flag");
        assert_eq!(plan.partitions, specs, "partition order must be kept");
    }

    #[test]
    fn toml_round_trip_preserves_partitions() {
        // ARRANGE
        let doc = sample();

        // ACT
        let serialized = doc.to_toml().expect("serialize");
        let parsed = Document::from_toml(&serialized).expect("deserialize");

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
            serialized.contains("api_version = \"muak.dev/diskplan/v1-beta\""),
            "serialized document must carry the api version: {serialized}"
        );
    }

    #[test]
    fn from_toml_rejects_unknown_api_version() {
        // ARRANGE
        let bytes = "api_version = \"muak.dev/diskplan/v999\"\nwipe = true\n[[partitions]]\nrole = \"esp\"\nname = \"EFI\"\ntype_guid = \"00000000-0000-0000-0000-000000000000\"\nsize = \"fill\"\n";

        // ACT
        let result = Document::from_toml(bytes);

        // ASSERT
        assert!(
            matches!(result, Err(DiskError::UnsupportedApiVersion(_))),
            "unknown api_version must be rejected"
        );
    }

    #[test]
    fn from_toml_rejects_missing_api_version() {
        // ARRANGE
        let bytes = "wipe = true\n[[partitions]]\nrole = \"esp\"\nname = \"EFI\"\ntype_guid = \"00000000-0000-0000-0000-000000000000\"\nsize = \"fill\"\n";

        // ACT
        let result = Document::from_toml(bytes);

        // ASSERT
        let error = result.expect_err("missing api_version must fail");
        assert!(error.to_string().contains("api_version"), "{error}");
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
        let doc = Document::new(
            true,
            vec![Partition {
                role: Role::Esp,
                name: "EFI".to_owned(),
                type_guid: guid(&ESP_TYPE_GUID),
                size: Size::Fill,
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
        let path = dir.path().join("diskplan.toml");
        let doc = sample();

        // ACT
        doc.write(&path).expect("write");
        let loaded = Document::read(&path).expect("read");

        // ASSERT
        assert_eq!(
            loaded, doc,
            "filesystem round trip must preserve the document"
        );
    }

    #[test]
    fn plan_conversion_round_trips() {
        // ARRANGE
        let plan = Layout::Uefi.plan();

        // ACT
        let doc = Document::from_plan(&plan).expect("from plan");
        let converted = doc.to_plan().expect("to plan");

        // ASSERT
        assert_eq!(converted, plan, "plan conversion must round trip");
    }

    #[test]
    fn plan_conversion_rejects_bad_type_guids() {
        // ARRANGE
        let doc = Document::new(
            true,
            vec![Partition {
                role: Role::Esp,
                name: "EFI".to_owned(),
                type_guid: "not-a-uuid".to_owned(),
                size: Size::Fill,
            }],
        );

        // ACT
        let result = doc.to_plan();

        // ASSERT
        assert!(result.is_err(), "invalid type GUIDs must be rejected");
    }
}
