use std::fs;
use std::path::Path;

fn read_modalias(device_path: &Path) -> Option<String> {
    let modalias_path = device_path.join("modalias");
    let modalias = fs::read_to_string(&modalias_path).ok()?;
    let modalias = modalias.trim();
    if modalias.is_empty() {
        return None;
    }
    Some(modalias.to_string())
}

pub fn for_each_modalias<F>(mut f: F) -> std::io::Result<()>
where
    F: FnMut(&str),
{
    for_each_modalias_in(Path::new("/sys/bus"), &mut f)
}

fn for_each_modalias_in<F>(sys_bus: &Path, f: &mut F) -> std::io::Result<()>
where
    F: FnMut(&str),
{
    if !sys_bus.exists() {
        return Ok(());
    }

    let devices_dirs: Vec<_> = fs::read_dir(sys_bus)?
        .filter_map(Result::ok)
        .map(|e| e.path().join("devices"))
        .filter(|p| p.exists())
        .collect();

    for devices_dir in devices_dirs {
        let Ok(entries) = fs::read_dir(&devices_dir) else {
            continue;
        };
        for modalias in entries
            .filter_map(Result::ok)
            .filter_map(|e| read_modalias(&e.path()))
        {
            f(&modalias);
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use std::io::Write;

    use tempfile::TempDir;

    use super::*;

    #[test]
    fn test_read_modalias_valid() {
        let dir = TempDir::new().expect("Failed to create temp dir");
        let modalias_path = dir.path().join("modalias");
        std::fs::write(&modalias_path, "pci:v00008086d00001234\n").expect("write failed");

        let result = read_modalias(dir.path());
        assert_eq!(result, Some("pci:v00008086d00001234".to_string()));
    }

    #[test]
    fn test_read_modalias_with_whitespace() {
        let dir = TempDir::new().expect("Failed to create temp dir");
        let modalias_path = dir.path().join("modalias");
        std::fs::write(&modalias_path, "  usb:v1234p5678  \n").expect("write failed");

        let result = read_modalias(dir.path());
        assert_eq!(result, Some("usb:v1234p5678".to_string()));
    }

    #[test]
    fn test_read_modalias_empty() {
        let dir = TempDir::new().expect("Failed to create temp dir");
        let modalias_path = dir.path().join("modalias");
        std::fs::write(&modalias_path, "").expect("write failed");

        let result = read_modalias(dir.path());
        assert_eq!(result, None);
    }

    #[test]
    fn test_read_modalias_no_file() {
        let dir = TempDir::new().expect("Failed to create temp dir");
        let result = read_modalias(dir.path());
        assert_eq!(result, None);
    }

    #[test]
    fn test_read_modalias_nonexistent_path() {
        let result = read_modalias(Path::new("/nonexistent/device/path"));
        assert_eq!(result, None);
    }

    fn create_mock_sysfs(base: &Path, buses: &[(&str, &[(&str, Option<&str>)])]) {
        for (bus_name, devices) in buses {
            let bus_dir = base.join(bus_name);
            let devices_dir = bus_dir.join("devices");
            std::fs::create_dir_all(&devices_dir).expect("create bus devices dir");

            for (device_name, modalias) in *devices {
                let device_dir = devices_dir.join(device_name);
                std::fs::create_dir_all(&device_dir).expect("create device dir");

                if let Some(alias) = modalias {
                    let modalias_path = device_dir.join("modalias");
                    let mut file = std::fs::File::create(&modalias_path).expect("create modalias");
                    writeln!(file, "{}", alias).expect("write modalias");
                }
            }
        }
    }

    #[test]
    fn test_for_each_modalias_empty_sysfs() {
        let dir = TempDir::new().expect("Failed to create temp dir");
        let mut collected = Vec::new();

        let result = for_each_modalias_in(dir.path(), &mut |m| {
            collected.push(m.to_string());
        });

        assert!(result.is_ok());
        assert!(collected.is_empty());
    }

    #[test]
    fn test_for_each_modalias_nonexistent_path() {
        let mut collected = Vec::new();

        let result = for_each_modalias_in(Path::new("/nonexistent/sys/bus"), &mut |m| {
            collected.push(m.to_string());
        });

        assert!(result.is_ok());
        assert!(collected.is_empty());
    }

    #[test]
    fn test_for_each_modalias_single_bus_single_device() {
        let dir = TempDir::new().expect("Failed to create temp dir");
        create_mock_sysfs(
            dir.path(),
            &[("pci", &[("0000:00:1f.0", Some("pci:v00008086d00001234"))])],
        );

        let mut collected = Vec::new();
        let result = for_each_modalias_in(dir.path(), &mut |m| {
            collected.push(m.to_string());
        });

        assert!(result.is_ok());
        assert_eq!(collected, vec!["pci:v00008086d00001234"]);
    }

    #[test]
    fn test_for_each_modalias_multiple_buses() {
        let dir = TempDir::new().expect("Failed to create temp dir");
        create_mock_sysfs(
            dir.path(),
            &[
                ("pci", &[("0000:00:1f.0", Some("pci:v00008086d00001234"))]),
                ("usb", &[("1-1", Some("usb:v046DpC52B"))]),
                ("acpi", &[("ACPI0003:00", Some("acpi:ACPI0003:"))]),
            ],
        );

        let mut collected = Vec::new();
        let result = for_each_modalias_in(dir.path(), &mut |m| {
            collected.push(m.to_string());
        });

        assert!(result.is_ok());
        assert_eq!(collected.len(), 3);
        assert!(collected.contains(&"pci:v00008086d00001234".to_string()));
        assert!(collected.contains(&"usb:v046DpC52B".to_string()));
        assert!(collected.contains(&"acpi:ACPI0003:".to_string()));
    }

    #[test]
    fn test_for_each_modalias_multiple_devices_per_bus() {
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

        let mut collected = Vec::new();
        let result = for_each_modalias_in(dir.path(), &mut |m| {
            collected.push(m.to_string());
        });

        assert!(result.is_ok());
        assert_eq!(collected.len(), 3);
    }

    #[test]
    fn test_for_each_modalias_device_without_modalias() {
        let dir = TempDir::new().expect("Failed to create temp dir");
        create_mock_sysfs(
            dir.path(),
            &[(
                "pci",
                &[
                    ("0000:00:1f.0", Some("pci:v00008086d00001234")),
                    ("0000:00:1f.1", None), // No modalias
                ],
            )],
        );

        let mut collected = Vec::new();
        let result = for_each_modalias_in(dir.path(), &mut |m| {
            collected.push(m.to_string());
        });

        assert!(result.is_ok());
        assert_eq!(collected.len(), 1);
        assert_eq!(collected[0], "pci:v00008086d00001234");
    }

    #[test]
    fn test_for_each_modalias_bus_without_devices_dir() {
        let dir = TempDir::new().expect("Failed to create temp dir");
        std::fs::create_dir_all(dir.path().join("pci")).expect("create bus dir");
        create_mock_sysfs(dir.path(), &[("usb", &[("1-1", Some("usb:v1234p5678"))])]);

        let mut collected = Vec::new();
        let result = for_each_modalias_in(dir.path(), &mut |m| {
            collected.push(m.to_string());
        });

        assert!(result.is_ok());
        assert_eq!(collected.len(), 1);
        assert_eq!(collected[0], "usb:v1234p5678");
    }

    #[test]
    fn test_for_each_modalias_empty_modalias() {
        let dir = TempDir::new().expect("Failed to create temp dir");
        let bus_dir = dir.path().join("pci").join("devices").join("0000:00:00.0");
        std::fs::create_dir_all(&bus_dir).expect("create dirs");
        std::fs::write(bus_dir.join("modalias"), "").expect("write empty");

        let mut collected = Vec::new();
        let result = for_each_modalias_in(dir.path(), &mut |m| {
            collected.push(m.to_string());
        });

        assert!(result.is_ok());
        assert!(collected.is_empty());
    }
}
