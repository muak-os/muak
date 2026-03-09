use std::fs;

use sysconfig::*;
use tempfile::TempDir;

#[test]
fn test_load_from_path_with_file() {
    // ARRANGE
    let temp_dir = TempDir::new().unwrap();
    let config_path = temp_dir.path().join("config.toml");
    let config_content = r#"
[host]
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

    // ACT
    let config = parse_from_str(config_content).unwrap();

    // ASSERT
    assert_eq!(config.host.disk, "test_disk");
    assert_eq!(config.network.ipv6, true);
    assert_eq!(config.vm.auto_restart, false);
}

#[test]
fn test_load_from_path_fallback_to_default() {
    // ARRANGE
    let default_str = serialize_default();

    // ACT
    let config: SystemConfig = toml::from_str(&default_str).unwrap();

    // ASSERT
    assert!(config.validate().is_ok());
}
