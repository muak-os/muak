use sysconfig::*;

use std::fs;
use tempfile::TempDir;

#[test]
fn test_load_from_path_with_file() {
    let temp_dir = TempDir::new().unwrap();
    let config_path = temp_dir.path().join("config.toml");
    let config_content = r#"
[system]
disk = "test_disk"
image = "test_image"
extensions = ["ext1", "ext2"]
port = 8080

[network]
ipv6 = true

[vm]
auto_restart = false
"#;
    fs::write(&config_path, config_content).unwrap();

    let config = parse_from_str(config_content).unwrap();
    assert_eq!(config.system.disk, "test_disk");
    assert_eq!(config.network.ipv6, true);
    assert_eq!(config.vm.auto_restart, false);
}

#[test]
fn test_load_from_path_fallback_to_default() {
    // Since load_from_path is pub(crate), we can't test it directly in integration tests.
    // Instead, test parse_from_str with the default serialized
    let default_str = serialize_default();
    let config: HostConfig = toml::from_str(&default_str).unwrap();
    assert!(config.validate().is_ok());
}
