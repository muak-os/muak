//! Platform firmware identity from EFI variables, SMBIOS, and devicetree.

use uefi::{Guid, guid};

use crate::nvram::{self, Id, VariableStore};
use crate::platform::{DevicetreeProperty, Platform, SmbiosAttribute};

/// Vendor GUID of Systemd Boot Loader Interface.
const LOADER_VENDOR: Guid = guid!("4a67b082-0a4c-41cf-b6c7-440b29bb8c4f");

/// `LoaderFirmwareInfo` variable, holding `<vendor> <major>.<minor>`.
const LOADER_FIRMWARE_INFO: Id = Id::new("LoaderFirmwareInfo", LOADER_VENDOR);

/// `LoaderFirmwareType` variable, holding the firmware family and revision.
const LOADER_FIRMWARE_TYPE: Id = Id::new("LoaderFirmwareType", LOADER_VENDOR);

/// Broad platform firmware family.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// UEFI firmware.
    Uefi,
    /// Legacy BIOS or CSM firmware.
    Bios,
    /// coreboot or a coreboot-based firmware.
    Coreboot,
    /// Device tree firmware without SMBIOS, such as U-Boot on arm64.
    Devicetree,
}

/// Where a firmware identity value was read from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Source {
    /// A `Loader*` EFI variable exported by the boot stub.
    EfiVariable,
    /// SMBIOS type 0, Platform Firmware Information.
    Smbios,
    /// The device tree.
    Devicetree,
}

/// System firmware identity.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Identity {
    /// Firmware vendor, when known.
    pub vendor: String,
    /// Firmware version, when known.
    pub version: String,
    /// Firmware release date, when known.
    pub date: String,
    /// Firmware family.
    pub kind: Kind,
    /// Where this identity was read from.
    pub source: Source,
}

/// Reads the platform firmware identity, preferring EFI variables.
#[must_use]
pub fn collect<S: VariableStore, P: Platform>(variables: &S, firmware: &P) -> Option<Identity> {
    let efi_boot = variables.is_boot();

    from_efi(variables, firmware, efi_boot)
        .or_else(|| from_smbios(firmware, efi_boot))
        .or_else(|| from_devicetree(firmware))
}

fn from_efi<S: VariableStore, P: Platform>(
    variables: &S,
    firmware: &P,
    efi_boot: bool,
) -> Option<Identity> {
    let info = nvram::read_string(variables, &LOADER_FIRMWARE_INFO)
        .ok()
        .flatten()?;

    let (vendor, version) = match info.rsplit_once(' ') {
        Some((vendor, version)) => (vendor.to_owned(), version.to_owned()),
        None => (info, String::new()),
    };

    Some(Identity {
        vendor,
        version,
        date: firmware.smbios(SmbiosAttribute::Date).unwrap_or_default(),
        kind: firmware_kind(variables, efi_boot),
        source: Source::EfiVariable,
    })
}

fn from_smbios<P: Platform>(firmware: &P, efi_boot: bool) -> Option<Identity> {
    let version = firmware.smbios(SmbiosAttribute::Version)?;
    let vendor = firmware.smbios(SmbiosAttribute::Vendor).unwrap_or_default();
    let kind = if vendor.to_ascii_lowercase().contains("coreboot") {
        Kind::Coreboot
    } else if efi_boot {
        Kind::Uefi
    } else {
        Kind::Bios
    };

    Some(Identity {
        vendor,
        version,
        date: firmware.smbios(SmbiosAttribute::Date).unwrap_or_default(),
        kind,
        source: Source::Smbios,
    })
}

fn from_devicetree<P: Platform>(firmware: &P) -> Option<Identity> {
    let model = firmware.devicetree(DevicetreeProperty::Model)?;
    let version = firmware
        .devicetree(DevicetreeProperty::Compatible)
        .unwrap_or_default();

    Some(Identity {
        vendor: model,
        version,
        date: String::new(),
        kind: Kind::Devicetree,
        source: Source::Devicetree,
    })
}

fn firmware_kind<S: VariableStore>(variables: &S, efi_boot: bool) -> Kind {
    match nvram::read_string(variables, &LOADER_FIRMWARE_TYPE)
        .ok()
        .flatten()
    {
        Some(kind) if kind.starts_with("BIOS") => Kind::Bios,
        Some(_kind) => Kind::Uefi,
        None if efi_boot => Kind::Uefi,
        None => Kind::Bios,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::Result;
    use crate::esrt::RawEntry;
    use crate::nvram::Update;

    #[derive(Default)]
    struct Fake {
        boot: bool,
        variables: Vec<(Id, Vec<u8>)>,
        smbios: Vec<(SmbiosAttribute, String)>,
        devicetree: Vec<(DevicetreeProperty, String)>,
    }

    impl Fake {
        fn boot(mut self) -> Self {
            self.boot = true;
            self
        }

        fn variable(mut self, id: Id, value: &str) -> Self {
            let mut bytes: Vec<u8> = value.encode_utf16().flat_map(u16::to_le_bytes).collect();
            bytes.extend_from_slice(&[0, 0]);
            self.variables.push((id, bytes));
            self
        }

        fn with_smbios(mut self, attribute: SmbiosAttribute, value: &str) -> Self {
            self.smbios.push((attribute, value.to_owned()));
            self
        }

        fn with_devicetree(mut self, property: DevicetreeProperty, value: &str) -> Self {
            self.devicetree.push((property, value.to_owned()));
            self
        }
    }

    impl VariableStore for Fake {
        fn is_boot(&self) -> bool {
            self.boot
        }

        fn is_available(&self) -> bool {
            true
        }

        fn prepare(&self) -> Result<bool> {
            Ok(true)
        }

        fn exists(&self, id: &Id) -> bool {
            self.variables.iter().any(|entry| entry.0 == *id)
        }

        fn read(&self, id: &Id) -> Result<Option<Vec<u8>>> {
            Ok(self
                .variables
                .iter()
                .find(|entry| entry.0 == *id)
                .map(|entry| entry.1.clone()))
        }

        fn write(&self, _update: Update<'_>) -> Result<()> {
            Ok(())
        }

        fn remove(&self, _id: &Id) -> Result<()> {
            Ok(())
        }
    }

    impl Platform for Fake {
        fn smbios(&self, attribute: SmbiosAttribute) -> Option<String> {
            self.smbios
                .iter()
                .find(|entry| entry.0 == attribute)
                .map(|entry| entry.1.clone())
        }

        fn devicetree(&self, property: DevicetreeProperty) -> Option<String> {
            self.devicetree
                .iter()
                .find(|entry| entry.0 == property)
                .map(|entry| entry.1.clone())
        }

        fn esrt(&self) -> Vec<RawEntry> {
            Vec::new()
        }
    }

    #[test]
    fn efi_variable_identity_wins() {
        // ARRANGE
        let fake = Fake::default()
            .boot()
            .variable(LOADER_FIRMWARE_INFO, "ACME Corp 5.31")
            .variable(LOADER_FIRMWARE_TYPE, "UEFI 2.10")
            .with_smbios(SmbiosAttribute::Date, "03/22/2024")
            .with_smbios(SmbiosAttribute::Vendor, "Should Not Win")
            .with_smbios(SmbiosAttribute::Version, "0");

        // ACT
        let identity = collect(&fake, &fake).expect("identity");

        // ASSERT
        assert_eq!(identity.vendor, "ACME Corp");
        assert_eq!(identity.version, "5.31");
        assert_eq!(identity.date, "03/22/2024");
        assert_eq!(identity.kind, Kind::Uefi);
        assert_eq!(identity.source, Source::EfiVariable);
    }

    #[test]
    fn smbios_is_used_when_no_efi_variable_exists() {
        // ARRANGE
        let fake = Fake::default()
            .boot()
            .with_smbios(SmbiosAttribute::Vendor, "coreboot")
            .with_smbios(SmbiosAttribute::Version, "24.02")
            .with_smbios(SmbiosAttribute::Date, "01/02/2024");

        // ACT
        let identity = collect(&fake, &fake).expect("identity");

        // ASSERT
        assert_eq!(identity.vendor, "coreboot");
        assert_eq!(identity.version, "24.02");
        assert_eq!(identity.kind, Kind::Coreboot);
        assert_eq!(identity.source, Source::Smbios);
    }

    #[test]
    fn devicetree_is_used_without_efi_or_smbios() {
        // ARRANGE
        let fake = Fake::default()
            .with_devicetree(DevicetreeProperty::Model, "Raspberry Pi 4 Model B")
            .with_devicetree(DevicetreeProperty::Compatible, "raspberrypi,4-model-b");

        // ACT
        let identity = collect(&fake, &fake).expect("identity");

        // ASSERT
        assert_eq!(identity.vendor, "Raspberry Pi 4 Model B");
        assert_eq!(identity.version, "raspberrypi,4-model-b");
        assert_eq!(identity.kind, Kind::Devicetree);
        assert_eq!(identity.source, Source::Devicetree);
    }

    #[test]
    fn absent_sources_yield_none() {
        // ARRANGE
        let fake = Fake::default();

        // ACT
        let identity = collect(&fake, &fake);

        // ASSERT
        assert!(identity.is_none(), "no source must yield no identity");
    }
}
