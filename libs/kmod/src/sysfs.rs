//! Sysfs modalias discovery.

use std::fs;
use std::path::Path;

/// Calls `callback` for each discovered sysfs modalias.
///
/// # Errors
///
/// Returns an error when the sysfs bus root cannot be enumerated.
pub fn for_each_modalias<F>(mut callback: F) -> std::io::Result<()>
where
    F: FnMut(&str),
{
    for_each_modalias_in(Path::new("/sys/bus"), &mut callback)
}

fn for_each_modalias_in<F>(sys_bus: &Path, callback: &mut F) -> std::io::Result<()>
where
    F: FnMut(&str),
{
    if !sys_bus.exists() {
        return Ok(());
    }

    let devices_dirs: Vec<_> = fs::read_dir(sys_bus)?
        .filter_map(Result::ok)
        .map(|entry| entry.path().join("devices"))
        .filter(|devices_dir| devices_dir.exists())
        .collect();

    for devices_dir in devices_dirs {
        let Ok(entries) = fs::read_dir(&devices_dir) else {
            continue;
        };
        for modalias in entries
            .filter_map(Result::ok)
            .filter_map(|entry| read_modalias(&entry.path()))
        {
            callback(&modalias);
        }
    }

    Ok(())
}

fn read_modalias(device_path: &Path) -> Option<String> {
    let modalias_path = device_path.join("modalias");
    let modalias = fs::read_to_string(&modalias_path).ok()?;
    let modalias = modalias.trim();
    if modalias.is_empty() {
        return None;
    }
    Some(modalias.to_owned())
}

#[cfg(test)]
mod tests {
    use std::io::Write as _;

    use tempfile::TempDir;

    use super::*;

    fn collect_modaliases(sys_bus: &Path) -> std::io::Result<Vec<String>> {
        let mut collected = Vec::new();
        for_each_modalias_in(sys_bus, &mut |modalias| {
            collected.push(modalias.to_owned());
        })?;
        Ok(collected)
    }

    #[test]
    fn read_modalias_valid() {
        // ARRANGE
        let dir = TempDir::new().expect("Failed to create temp dir");
        let modalias_path = dir.path().join("modalias");
        std::fs::write(&modalias_path, "pci:v00008086d00001234\n").expect("write failed");

        // ACT
        let result = read_modalias(dir.path());

        // ASSERT
        assert_eq!(result, Some("pci:v00008086d00001234".to_owned()));
    }

    #[test]
    fn read_modalias_with_whitespace() {
        // ARRANGE
        let dir = TempDir::new().expect("Failed to create temp dir");
        let modalias_path = dir.path().join("modalias");
        std::fs::write(&modalias_path, "  usb:v1234p5678  \n").expect("write failed");

        // ACT
        let result = read_modalias(dir.path());

        // ASSERT
        assert_eq!(result, Some("usb:v1234p5678".to_owned()));
    }

    #[test]
    fn read_modalias_empty() {
        // ARRANGE
        let dir = TempDir::new().expect("Failed to create temp dir");
        let modalias_path = dir.path().join("modalias");
        std::fs::write(&modalias_path, "").expect("write failed");

        // ACT
        let result = read_modalias(dir.path());

        // ASSERT
        assert_eq!(result, None);
    }

    #[test]
    fn read_modalias_no_file() {
        // ARRANGE
        let dir = TempDir::new().expect("Failed to create temp dir");

        // ACT
        let result = read_modalias(dir.path());

        // ASSERT
        assert_eq!(result, None);
    }

    #[test]
    fn read_modalias_nonexistent_path() {
        // ACT
        let result = read_modalias(Path::new("/nonexistent/device/path"));

        // ASSERT
        assert_eq!(result, None);
    }

    type MockDevices<'a> = &'a [(&'a str, Option<&'a str>)];

    fn create_mock_sysfs(base: &Path, buses: &[(&str, MockDevices<'_>)]) {
        for &(bus_name, devices) in buses {
            create_mock_bus(base, bus_name, devices);
        }
    }

    fn create_mock_bus(base: &Path, bus_name: &str, devices: MockDevices<'_>) {
        let bus_devices_dir = base.join(bus_name).join("devices");
        std::fs::create_dir_all(&bus_devices_dir).expect("create bus devices dir");

        for &(device_name, modalias) in devices {
            create_mock_device(&bus_devices_dir, device_name, modalias);
        }
    }

    fn create_mock_device(devices_dir: &Path, device_name: &str, modalias: Option<&str>) {
        let mock_device_dir = devices_dir.join(device_name);
        std::fs::create_dir_all(&mock_device_dir).expect("create device dir");

        let Some(alias) = modalias else {
            return;
        };

        let modalias_path = mock_device_dir.join("modalias");
        let mut file = std::fs::File::create(&modalias_path).expect("create modalias");
        writeln!(file, "{alias}").expect("write modalias");
    }

    #[test]
    fn for_each_modalias_empty_sysfs() {
        // ARRANGE
        let dir = TempDir::new().expect("Failed to create temp dir");

        // ACT
        let collected = collect_modaliases(dir.path()).expect("collect failed");

        // ASSERT
        assert!(collected.is_empty());
    }

    #[test]
    fn for_each_modalias_nonexistent_path() {
        // ARRANGE
        // ACT
        let collected =
            collect_modaliases(Path::new("/nonexistent/sys/bus")).expect("collect failed");

        // ASSERT
        assert!(collected.is_empty());
    }

    #[test]
    fn for_each_modalias_single_bus_single_device() {
        // ARRANGE
        let dir = TempDir::new().expect("Failed to create temp dir");
        create_mock_sysfs(
            dir.path(),
            &[("pci", &[("0000:00:1f.0", Some("pci:v00008086d00001234"))])],
        );

        // ACT
        let collected = collect_modaliases(dir.path()).expect("collect failed");

        // ASSERT
        assert_eq!(collected, vec!["pci:v00008086d00001234"]);
    }

    #[test]
    fn for_each_modalias_multiple_buses() {
        // ARRANGE
        let dir = TempDir::new().expect("Failed to create temp dir");
        create_mock_sysfs(
            dir.path(),
            &[
                ("pci", &[("0000:00:1f.0", Some("pci:v00008086d00001234"))]),
                ("usb", &[("1-1", Some("usb:v046DpC52B"))]),
                ("acpi", &[("ACPI0003:00", Some("acpi:ACPI0003:"))]),
            ],
        );

        // ACT
        let collected = collect_modaliases(dir.path()).expect("collect failed");

        // ASSERT
        assert_eq!(collected.len(), 3);
        assert!(
            collected
                .iter()
                .any(|alias| alias == "pci:v00008086d00001234")
        );
        assert!(collected.iter().any(|alias| alias == "usb:v046DpC52B"));
        assert!(collected.iter().any(|alias| alias == "acpi:ACPI0003:"));
    }

    #[test]
    fn for_each_modalias_multiple_devices_per_bus() {
        // ARRANGE
        let dir = TempDir::new().expect("Failed to create temp dir");
        create_mock_sysfs(
            dir.path(),
            &[(
                "pci",
                &[
                    ("0000:00:1f.0", Some("pci:v00008086d00001234")),
                    ("0000:00:1f.1", Some("pci:v00008086d00005678")),
                    ("0000:01:00.0", Some("pci:v000010DEd00001234")),
                ],
            )],
        );

        // ACT
        let collected = collect_modaliases(dir.path()).expect("collect failed");

        // ASSERT
        assert_eq!(collected.len(), 3);
    }

    #[test]
    fn for_each_modalias_device_without_modalias() {
        // ARRANGE
        let dir = TempDir::new().expect("Failed to create temp dir");
        create_mock_sysfs(
            dir.path(),
            &[(
                "pci",
                &[
                    ("0000:00:1f.0", Some("pci:v00008086d00001234")),
                    ("0000:00:1f.1", None),
                ],
            )],
        );

        // ACT
        let collected = collect_modaliases(dir.path()).expect("collect failed");

        // ASSERT
        assert_eq!(collected.len(), 1);
        assert_eq!(
            collected.first().expect("first modalias"),
            "pci:v00008086d00001234"
        );
    }

    #[test]
    fn for_each_modalias_bus_without_devices_dir() {
        // ARRANGE
        let dir = TempDir::new().expect("Failed to create temp dir");
        std::fs::create_dir_all(dir.path().join("pci")).expect("create bus dir");
        create_mock_sysfs(dir.path(), &[("usb", &[("1-1", Some("usb:v1234p5678"))])]);

        // ACT
        let collected = collect_modaliases(dir.path()).expect("collect failed");

        // ASSERT
        assert_eq!(collected.len(), 1);
        assert_eq!(collected.first().expect("first modalias"), "usb:v1234p5678");
    }

    #[test]
    fn for_each_modalias_empty_modalias() {
        // ARRANGE
        let dir = TempDir::new().expect("Failed to create temp dir");
        let bus_dir = dir.path().join("pci").join("devices").join("0000:00:00.0");
        std::fs::create_dir_all(&bus_dir).expect("create dirs");
        std::fs::write(bus_dir.join("modalias"), "").expect("write empty");

        // ACT
        let collected = collect_modaliases(dir.path()).expect("collect failed");

        // ASSERT
        assert!(collected.is_empty());
    }

    #[test]
    fn for_each_modalias_skips_unreadable_devices_dir() {
        // ARRANGE
        let dir = TempDir::new().expect("Failed to create temp dir");
        std::fs::create_dir_all(dir.path().join("pci")).expect("create bus dir");
        std::fs::write(dir.path().join("pci").join("devices"), b"not a directory")
            .expect("write devices file");

        // ACT
        let collected = collect_modaliases(dir.path()).expect("collect failed");

        // ASSERT
        assert!(collected.is_empty());
    }

    #[test]
    fn public_for_each_modalias_reads_live_sysfs() {
        // ARRANGE
        let mut collected = Vec::new();

        // ACT
        let result = for_each_modalias(|modalias| {
            collected.push(modalias.to_owned());
        });

        // ASSERT
        result.expect("live sysfs scan should not fail");
    }
}
