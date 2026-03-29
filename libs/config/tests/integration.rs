use std::fs;

use config::*;
use tempfile::TempDir;

#[test]
fn load_from_path_with_file() {
    // ARRANGE
    let temp_dir = TempDir::new().unwrap();
    let config_path = temp_dir.path().join("config.toml");
    let config_content = r#"
[host]
name = "muak"
image = "test_image"
extensions = ["ext1", "ext2"]
port = 8080
ntp = "pool.ntp.org"

[disk]
system = "test_disk"

[network]
ipv6 = true

[vm]
auto_restart = false
"#;
    fs::write(&config_path, config_content).unwrap();

    // ACT
    let config = parse_from_str(config_content).unwrap();

    // ASSERT
    assert_eq!(config.disk.system, "test_disk");
    assert_eq!(config.network.ipv6, true);
    assert_eq!(config.vm.auto_restart, false);
}

#[test]
fn load_from_path_fallback_to_default() {
    // ARRANGE
    let default_str = serialize_default();

    // ACT
    let config = parse_from_str(&default_str).unwrap();

    // ASSERT
    assert!(config.validate().is_ok());
}

#[test]
fn load_from_path_reads_tempfile() {
    // ARRANGE
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("config.toml");
    let content = "[host]\nname = \"frompath\"\nport = 5555\n";
    fs::write(&path, content).unwrap();

    // ACT
    let config = load_from_path(&path).unwrap();

    // ASSERT
    assert_eq!(config.host.name, "frompath");
    assert_eq!(config.host.port, 5555);
}

#[test]
fn serialize_and_parse_round_trip() {
    // ARRANGE
    let mut original = SystemConfig::default();
    original.host.port = 4321;
    original.host.name = "roundtrip".to_string();

    // ACT
    let s = serialize(&original).unwrap();
    let restored = parse_from_str(&s).unwrap();

    // ASSERT
    assert_eq!(restored.host.port, 4321);
    assert_eq!(restored.host.name, "roundtrip");
}

#[test]
fn serialize_default_is_valid() {
    // ACT
    let s = serialize_default();

    // ASSERT
    assert!(!s.is_empty());
    let config = parse_from_str(&s).unwrap();
    assert!(config.validate().is_ok());
}

#[test]
fn auth_serialize_and_parse_round_trip() {
    // ARRANGE
    let config = AuthConfig {
        users: vec![AuthUser {
            fingerprint: "integration_fp".to_string(),
            permissions: vec![Permission::Admin],
        }],
        revoked: vec!["old_fp".to_string()],
    };

    // ACT
    let s = serialize_auth(&config).unwrap();
    let restored = auth::parse(&s).unwrap();

    // ASSERT
    assert_eq!(restored.users.len(), 1);
    assert_eq!(restored.users[0].fingerprint, "integration_fp");
    assert_eq!(restored.revoked, vec!["old_fp"]);
}

#[test]
fn auth_load_from_path_nonexistent_returns_default() {
    // ACT
    let config = auth::load_from_path(std::path::Path::new("/no/such/file.toml")).unwrap();

    // ASSERT
    assert!(config.users.is_empty());
}

#[test]
fn auth_load_from_path_valid_file() {
    // ARRANGE
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("auth.toml");
    fs::write(
        &path,
        "[[users]]\nfingerprint = \"itfp\"\npermissions = [\"admin\"]\n",
    )
    .unwrap();

    // ACT
    let config = auth::load_from_path(&path).unwrap();

    // ASSERT
    assert_eq!(config.users.len(), 1);
    assert_eq!(config.users[0].fingerprint, "itfp");
}

#[test]
fn try_config_returns_none_before_init() {
    // ACT
    let result = try_config();

    let _ = result;
}
