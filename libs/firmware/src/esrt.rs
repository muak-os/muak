//! ESRT (EFI System Resource Table) components.

use crate::platform::Platform;

/// Raw ESRT entry as reported by a [`Platform`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawEntry {
    /// Firmware class GUID identifying the component.
    pub class: String,
    /// Raw `fw_type` value.
    pub kind: u32,
    /// Current firmware version, as reported by the firmware.
    pub version: u32,
    /// Lowest version the firmware accepts, when reported.
    pub lowest: Option<u32>,
    /// Version of the last attempted update, when reported.
    pub last_attempt: Option<u32>,
    /// Raw `last_attempt_status` value, when reported.
    pub last_status: Option<u32>,
}

/// Kind of updatable firmware component.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ComponentKind {
    /// Unknown component type.
    Unknown,
    /// System firmware.
    System,
    /// Device firmware.
    Device,
    /// UEFI driver.
    Driver,
}

impl ComponentKind {
    /// Maps the raw ESRT `fw_type` value.
    fn from_raw(value: u32) -> Self {
        match value {
            1 => Self::System,
            2 => Self::Device,
            3 => Self::Driver,
            _ => Self::Unknown,
        }
    }
}

/// Result of the last firmware update attempt.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AttemptStatus {
    /// The update attempt succeeded.
    Success,
    /// Insufficient resources.
    InsufficientResources,
    /// Incorrect version.
    IncorrectVersion,
    /// Invalid format.
    InvalidFormat,
    /// Authentication error.
    AuthenticationError,
    /// AC power event.
    AcPower,
    /// Battery power event.
    BatteryPower,
    /// Unrecognized status.
    Unknown,
}

impl AttemptStatus {
    /// Maps the raw ESRT `last_attempt_status` value.
    fn from_raw(value: u32) -> Self {
        match value {
            0 => Self::Success,
            1 => Self::InsufficientResources,
            2 => Self::IncorrectVersion,
            3 => Self::InvalidFormat,
            4 => Self::AuthenticationError,
            5 => Self::AcPower,
            6 => Self::BatteryPower,
            _ => Self::Unknown,
        }
    }
}

/// One updatable firmware component reported by the firmware.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Component {
    /// Firmware class GUID identifying the component.
    pub class: String,
    /// Component type.
    pub kind: ComponentKind,
    /// Current firmware version, as reported by the firmware.
    pub version: u32,
    /// Display form of the current version.
    pub version_text: String,
    /// Lowest version the firmware accepts, when reported.
    pub lowest: Option<u32>,
    /// Version of the last attempted update, when reported.
    pub last_attempt: Option<u32>,
    /// Result of the last attempted update, when reported.
    pub last_status: Option<AttemptStatus>,
}

impl Component {
    /// Maps a raw backend entry into a component.
    fn from_raw(raw: RawEntry) -> Self {
        Self {
            class: raw.class,
            kind: ComponentKind::from_raw(raw.kind),
            version: raw.version,
            version_text: raw.version.to_string(),
            lowest: raw.lowest,
            last_attempt: raw.last_attempt,
            last_status: raw.last_status.map(AttemptStatus::from_raw),
        }
    }
}

/// Maps a platform's raw ESRT entries into components, sorted by class.
#[must_use]
pub fn collect<P: Platform>(platform: &P) -> Vec<Component> {
    let mut components: Vec<Component> = platform
        .esrt()
        .into_iter()
        .map(Component::from_raw)
        .collect();

    components.sort_by(|left, right| left.class.cmp(&right.class));

    components
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::platform::{DevicetreeProperty, SmbiosAttribute};

    struct Fake {
        entries: Vec<RawEntry>,
    }

    impl Platform for Fake {
        fn smbios(&self, _attribute: SmbiosAttribute) -> Option<String> {
            None
        }

        fn devicetree(&self, _property: DevicetreeProperty) -> Option<String> {
            None
        }

        fn esrt(&self) -> Vec<RawEntry> {
            self.entries.clone()
        }
    }

    fn entry(class: &str, kind: u32, version: u32) -> RawEntry {
        RawEntry {
            class: class.to_owned(),
            kind,
            version,
            lowest: None,
            last_attempt: None,
            last_status: None,
        }
    }

    #[test]
    fn maps_every_entry_field() {
        // ARRANGE
        let fake = Fake {
            entries: vec![RawEntry {
                class: "6f5f2d1e-0000-4000-8000-000000000000".to_owned(),
                kind: 1,
                version: 26,
                lowest: Some(1),
                last_attempt: Some(25),
                last_status: Some(0),
            }],
        };

        // ACT
        let components = collect(&fake);

        // ASSERT
        assert_eq!(components.len(), 1);
        let component = components.first().expect("component");
        assert_eq!(component.class, "6f5f2d1e-0000-4000-8000-000000000000");
        assert_eq!(component.kind, ComponentKind::System);
        assert_eq!(component.version, 26);
        assert_eq!(component.version_text, "26");
        assert_eq!(component.lowest, Some(1));
        assert_eq!(component.last_attempt, Some(25));
        assert_eq!(component.last_status, Some(AttemptStatus::Success));
    }

    #[test]
    fn maps_unknown_values_to_unknown() {
        // ARRANGE
        let fake = Fake {
            entries: vec![RawEntry {
                class: "abc".to_owned(),
                kind: 99,
                version: 1,
                lowest: None,
                last_attempt: None,
                last_status: Some(99),
            }],
        };

        // ACT
        let components = collect(&fake);

        // ASSERT
        let component = components.first().expect("component");
        assert_eq!(component.kind, ComponentKind::Unknown);
        assert_eq!(component.last_status, Some(AttemptStatus::Unknown));
    }

    #[test]
    fn no_entries_yields_no_components() {
        // ARRANGE
        let fake = Fake {
            entries: Vec::new(),
        };

        // ACT
        let components = collect(&fake);

        // ASSERT
        assert!(components.is_empty(), "no entries means no components");
    }

    #[test]
    fn components_are_sorted_by_class() {
        // ARRANGE
        let fake = Fake {
            entries: vec![entry("bbbb", 2, 2), entry("aaaa", 1, 1)],
        };

        // ACT
        let components = collect(&fake);

        // ASSERT
        let classes: Vec<&str> = components
            .iter()
            .map(|entry| entry.class.as_str())
            .collect();
        assert_eq!(classes, vec!["aaaa", "bbbb"]);
    }
}
