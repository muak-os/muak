//! The partition plan: the executor model applied to one disk.

use serde::{Deserialize, Serialize};

use crate::role::Role;

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
#[derive(Debug, Clone, PartialEq, Eq)]
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
    #[must_use = "dropping it discards the partition spec"]
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
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Plan {
    /// Whether the disk is wiped before applying the plan.
    pub wipe: bool,
    /// Planned partitions, in placement order.
    pub partitions: Vec<PartitionSpec>,
}

impl Plan {
    /// Creates a plan that wipes the disk before placing the partitions.
    #[must_use = "dropping it discards the planned specs"]
    pub fn wiped(partitions: Vec<PartitionSpec>) -> Self {
        Self {
            wipe: true,
            partitions,
        }
    }
}
