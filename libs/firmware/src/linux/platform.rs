//! Linux sysfs platform data.

use std::fs;
use std::path::Path;

use super::Linux;
use crate::esrt::RawEntry;
use crate::platform::{DevicetreeProperty, Platform, SmbiosAttribute};

impl Platform for Linux {
    fn smbios(&self, attribute: SmbiosAttribute) -> Option<String> {
        let file = match attribute {
            SmbiosAttribute::Vendor => "bios_vendor",
            SmbiosAttribute::Version => "bios_version",
            SmbiosAttribute::Date => "bios_date",
        };

        read_trimmed(&self.sysroot.join("class").join("dmi").join("id").join(file))
    }

    fn devicetree(&self, property: DevicetreeProperty) -> Option<String> {
        let file = match property {
            DevicetreeProperty::Model => "model",
            DevicetreeProperty::Compatible => "compatible",
        };
        let base = self
            .sysroot
            .join("firmware")
            .join("devicetree")
            .join("base");
        let value = read_trimmed(&base.join(file))?;

        match property {
            DevicetreeProperty::Compatible => value.split('\0').next().map(str::to_owned),
            DevicetreeProperty::Model => Some(value),
        }
    }

    fn esrt(&self) -> Vec<RawEntry> {
        let dir = self
            .sysroot
            .join("firmware")
            .join("efi")
            .join("esrt")
            .join("entries");
        let Ok(read_dir) = fs::read_dir(dir) else {
            return Vec::new();
        };

        read_dir
            .filter_map(std::result::Result::ok)
            .filter(|entry| entry.file_type().is_ok_and(|kind| kind.is_dir()))
            .filter_map(|entry| raw_entry(&entry.path()))
            .collect()
    }
}

fn raw_entry(dir: &Path) -> Option<RawEntry> {
    let class = read_trimmed(&dir.join("fw_class"))?;
    if class.is_empty() {
        return None;
    }

    Some(RawEntry {
        class,
        kind: read_number(&dir.join("fw_type")).unwrap_or(0),
        version: read_number(&dir.join("fw_version"))?,
        lowest: read_number(&dir.join("lowest_supported_fw_version")),
        last_attempt: read_number(&dir.join("last_attempt_version")),
        last_status: read_number(&dir.join("last_attempt_status")),
    })
}

fn read_trimmed(path: &Path) -> Option<String> {
    let content = fs::read_to_string(path).ok()?;

    Some(
        content
            .trim_matches(|character: char| character.is_whitespace() || character == '\0')
            .to_owned(),
    )
}

fn read_number(path: &Path) -> Option<u32> {
    read_trimmed(path)?.parse().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rooted(name: &str) -> (tempfile::TempDir, Linux) {
        let root = tempfile::Builder::new()
            .prefix(&format!("firmware-{name}-"))
            .tempdir()
            .expect("tempdir");

        let system = Linux::rooted(root.path());

        (root, system)
    }

    fn write(root: &Path, relative: &str, value: &str) {
        let path = root.join(relative);
        fs::create_dir_all(path.parent().expect("parent")).expect("create parent");
        fs::write(path, value).expect("write attribute");
    }

    #[test]
    fn smbios_reads_dmi_attribute() {
        // ARRANGE
        let (root, system) = rooted("smbios");
        write(root.path(), "class/dmi/id/bios_version", "24.02\n");

        // ACT
        let version = system.smbios(SmbiosAttribute::Version);

        // ASSERT
        assert_eq!(version.as_deref(), Some("24.02"));
    }

    #[test]
    fn devicetree_reads_model_and_first_compatible() {
        // ARRANGE
        let (root, system) = rooted("devicetree");
        write(
            root.path(),
            "firmware/devicetree/base/model",
            "Raspberry Pi 4 Model B\0",
        );
        write(
            root.path(),
            "firmware/devicetree/base/compatible",
            "raspberrypi,4-model-b\0brcm,bcm2711\0",
        );

        // ACT
        let model = system.devicetree(DevicetreeProperty::Model);
        let compatible = system.devicetree(DevicetreeProperty::Compatible);

        // ASSERT
        assert_eq!(model.as_deref(), Some("Raspberry Pi 4 Model B"));
        assert_eq!(compatible.as_deref(), Some("raspberrypi,4-model-b"));
    }

    #[test]
    fn esrt_reads_raw_entries() {
        // ARRANGE
        let (root, system) = rooted("esrt");
        let base = root.path().join("firmware/efi/esrt/entries/entry0");
        write(&base, "fw_class", "class-a");
        write(&base, "fw_type", "1");
        write(&base, "fw_version", "26");

        // ACT
        let entries = system.esrt();

        // ASSERT
        assert_eq!(entries.len(), 1);
        let entry = entries.first().expect("entry");
        assert_eq!(entry.class, "class-a");
        assert_eq!(entry.kind, 1);
        assert_eq!(entry.version, 26);
    }

    #[test]
    fn missing_sources_yield_none_and_empty() {
        // ARRANGE
        let (_root, system) = rooted("missing");

        // ACT
        let version = system.smbios(SmbiosAttribute::Version);
        let model = system.devicetree(DevicetreeProperty::Model);
        let entries = system.esrt();

        // ASSERT
        assert!(version.is_none(), "missing SMBIOS must yield none");
        assert!(model.is_none(), "missing devicetree must yield none");
        assert!(entries.is_empty(), "missing ESRT must yield no entries");
    }
}
