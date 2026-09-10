//! Functional role of a partition, independent of its name or slot.

use serde::{Deserialize, Serialize};

/// Functional role of a partition, independent of its name or slot.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    /// EFI system partition holding boot assets.
    Esp,
    /// Encrypted state partition holding configuration and secrets.
    State,
    /// Data partition holding workload data.
    Data,
}

impl Role {
    /// GPT partition name for this role.
    #[must_use]
    pub const fn gpt_name(self) -> &'static str {
        match self {
            Self::Esp => "EFI",
            Self::State => "STATE",
            Self::Data => "DATA",
        }
    }

    /// Device-mapper name for this role's encrypted volume.
    #[must_use]
    pub const fn dm_name(self) -> &'static str {
        match self {
            Self::Esp => "muak-esp",
            Self::State => "muak-state",
            Self::Data => "muak-data",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gpt_names_are_stable() {
        // ARRANGE / ACT / ASSERT
        assert_eq!(Role::Esp.gpt_name(), "EFI");
        assert_eq!(Role::State.gpt_name(), "STATE");
        assert_eq!(Role::Data.gpt_name(), "DATA");
    }

    #[test]
    fn dm_names_are_stable() {
        // ARRANGE / ACT / ASSERT
        assert_eq!(Role::State.dm_name(), "muak-state");
        assert_eq!(Role::Data.dm_name(), "muak-data");
    }

    #[test]
    fn serde_round_trip_uses_lowercase() {
        // ARRANGE
        #[derive(Deserialize)]
        struct Wrapper {
            role: Role,
        }

        // ACT
        let parsed: Wrapper = toml::from_str("role = \"state\"").expect("parse");

        // ASSERT
        assert_eq!(parsed.role, Role::State, "role must deserialize lowercase");
    }
}
