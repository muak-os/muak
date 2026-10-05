#[cfg(test)]
mod tests {
    use std::fs;

    use config::auth::{self, State, User};
    use config::permission::Permission;
    use config::system::{self, Config};
    use tempfile::TempDir;

    #[test]
    fn load_from_path_with_file() {
        // ARRANGE
        let temp_dir = TempDir::new().unwrap();
        let config_path = temp_dir.path().join("config.toml");
        let config_content = r#"
    api_version = "muak.dev/config/v1-beta"

    [host]
    name = "muak"
    registry = "ghcr.io/muak-os"
    channel = "stable"
    version = "v1.0.0"
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
        let config = system::parse_from_str(config_content).unwrap();

        // ASSERT
        assert_eq!(config.disk.system, "test_disk");
        assert!(config.network.ipv6);
        assert!(!config.vm.auto_restart);
    }

    #[test]
    fn load_from_path_fallback_to_default() {
        // ARRANGE
        let default_str = system::serialize_default();

        // ACT
        let config = system::parse_from_str(&default_str).unwrap();

        // ASSERT
        config.validate().unwrap();
    }

    #[test]
    fn load_from_path_reads_tempfile() {
        // ARRANGE
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("config.toml");
        let content =
            "api_version = \"muak.dev/config/v1-beta\"\n[host]\nname = \"frompath\"\nport = 5555\n";
        fs::write(&path, content).unwrap();

        // ACT
        let config = system::load_from_path(&path).unwrap();

        // ASSERT
        assert_eq!(config.host.name, "frompath");
        assert_eq!(config.host.port, 5555);
    }

    #[test]
    fn serialize_and_parse_round_trip() {
        // ARRANGE
        let mut original = Config::default();
        original.host.port = 4321;
        original.host.name = "roundtrip".to_owned();

        // ACT
        let serialized = system::serialize(&original).unwrap();
        let restored = system::parse_from_str(&serialized).unwrap();

        // ASSERT
        assert_eq!(restored.host.port, 4321);
        assert_eq!(restored.host.name, "roundtrip");
    }

    #[test]
    fn serialize_default_is_valid() {
        // ACT
        let serialized = system::serialize_default();

        // ASSERT
        assert_ne!(serialized, "");
        let config = system::parse_from_str(&serialized).unwrap();
        config.validate().unwrap();
    }

    #[test]
    fn auth_serialize_and_parse_round_trip() {
        // ARRANGE
        let config = State {
            users: vec![User {
                fingerprint: "integration_fp".to_owned(),
                permissions: vec![Permission::Admin],
            }],
            revoked: vec!["old_fp".to_owned()],
        };

        // ACT
        let serialized = auth::serialize(&config).unwrap();
        let restored = auth::parse(&serialized).unwrap();

        // ASSERT
        assert_eq!(restored.users.len(), 1);
        assert_eq!(
            restored.users.first().map(|user| user.fingerprint.as_str()),
            Some("integration_fp")
        );
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
        assert_eq!(
            config.users.first().map(|user| user.fingerprint.as_str()),
            Some("itfp")
        );
    }

    #[test]
    fn try_config_returns_none_before_init() {
        // ACT & ASSERT
        assert!(system::try_config().is_none());
    }
}
