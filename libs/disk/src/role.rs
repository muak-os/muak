//! Functional role of a partition, independent of its name or slot.

use parttable::gpt::partition::LINUX_FS_GUID;
use serde::{Deserialize, Serialize};

use crate::sizes::ESP_TYPE_GUID;

/// Functional role of a partition, independent of its name or slot.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    /// EFI system partition holding boot assets.
    Esp,
    /// Encrypted state partition holding configuration and secrets.
    State,
    /// Encrypted ephemeral partition holding resettable workload data.
    Ephemeral,
    /// Data partition holding workload data.
    Data,
}

impl Role {
    /// GPT partition type GUID for this role (wire byte order).
    #[must_use]
    pub const fn type_guid(self) -> [u8; 16] {
        match self {
            Self::Esp => ESP_TYPE_GUID,
            Self::State | Self::Ephemeral | Self::Data => LINUX_FS_GUID,
        }
    }

    /// GPT partition name for this role.
    #[must_use]
    pub const fn gpt_name(self) -> &'static str {
        match self {
            Self::Esp => "EFI",
            Self::State => "STATE",
            Self::Ephemeral => "EPHEMERAL",
            Self::Data => "DATA",
        }
    }

    /// Role carrying `gpt_name`, or `None` when the name is not a Muak role.
    #[must_use]
    pub fn from_gpt_name(gpt_name: &str) -> Option<Self> {
        [Self::Esp, Self::State, Self::Ephemeral, Self::Data]
            .into_iter()
            .find(|role| role.gpt_name() == gpt_name)
    }

    /// Device-mapper name for this role's encrypted volume.
    #[must_use]
    pub const fn dm_name(self) -> &'static str {
        match self {
            Self::Esp => "muak-esp",
            Self::State => "muak-state",
            Self::Ephemeral => "muak-ephemeral",
            Self::Data => "muak-data",
        }
    }
}
