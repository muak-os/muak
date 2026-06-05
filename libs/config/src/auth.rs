//! Mutable authentication and authorization state.

use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, OnceLock, RwLock};

use serde::{Deserialize, Serialize};

use crate::Permission;
use crate::codec::{Codec, TomlCodec};
use crate::error::Result;

/// Path to the auth state file on disk.
pub const AUTH_PATH: &str = "/run/state/auth.toml";
/// File extension for auth state files.
pub const AUTH_EXTENSION: &str = "toml";

/// Cached auth state with mtime-based invalidation.
struct AuthCache {
    data: RwLock<Arc<AuthConfig>>,
    mtime: AtomicU64,
}

static AUTH_CACHE: OnceLock<AuthCache> = OnceLock::new();

/// Authentication and authorization configuration.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct AuthConfig {
    /// Registered users with their permissions.
    pub users: Vec<AuthUser>,
    /// Revoked certificate fingerprints.
    pub revoked: Vec<String>,
}

/// An authorized user identified by certificate fingerprint.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthUser {
    /// Certificate fingerprint identifying this user.
    pub fingerprint: String,
    /// Permissions granted to this user.
    pub permissions: Vec<Permission>,
}

/// Initializes the auth cache. Called once at startup by [`crate::init()`].
///
/// Returns an error if `auth.toml` exists but cannot be parsed.
pub(crate) fn init() -> Result<()> {
    let path = Path::new(AUTH_PATH);
    let (config, mtime) = if path.exists() {
        let config = load_from_path(path)?;
        let mtime = file_mtime(AUTH_PATH);
        (config, mtime)
    } else {
        (AuthConfig::default(), 0)
    };

    let cache = AuthCache {
        data: RwLock::new(Arc::new(config)),
        mtime: AtomicU64::new(mtime),
    };
    let _ = AUTH_CACHE.set(cache);
    Ok(())
}

/// Returns the current auth config, reloading from disk if the file changed.
///
/// # Panics
///
/// Panics if [`crate::init()`] has not been called.
pub fn auth() -> Arc<AuthConfig> {
    let cache = AUTH_CACHE.get().expect("Auth not initialized");

    let current_mtime = file_mtime(AUTH_PATH);
    let cached_mtime = cache.mtime.load(Ordering::Relaxed);

    if current_mtime != cached_mtime {
        let (config, mtime) = load_with_mtime();
        let new = Arc::new(config);
        if let Ok(mut data) = cache.data.write() {
            *data = Arc::clone(&new);
            cache.mtime.store(mtime, Ordering::Relaxed);
        }
        return new;
    }

    let data = cache.data.read().expect("auth lock poisoned");
    Arc::clone(&data)
}

/// Returns the current auth config, or `None` if [`crate::init()`] hasn't been called.
pub fn try_auth() -> Option<Arc<AuthConfig>> {
    let cache = AUTH_CACHE.get()?;

    let current_mtime = file_mtime(AUTH_PATH);
    let cached_mtime = cache.mtime.load(Ordering::Relaxed);

    if current_mtime != cached_mtime {
        let (config, mtime) = load_with_mtime();
        let new = Arc::new(config);
        if let Ok(mut data) = cache.data.write() {
            *data = Arc::clone(&new);
            cache.mtime.store(mtime, Ordering::Relaxed);
        }
        return Some(new);
    }

    let data = cache.data.read().ok()?;
    Some(Arc::clone(&data))
}

/// Serializes an [`AuthConfig`] to a string.
pub fn serialize(config: &AuthConfig) -> Result<String> {
    TomlCodec::encode(config)
}

/// Parses an [`AuthConfig`] from a string.
pub fn parse(contents: &str) -> Result<AuthConfig> {
    TomlCodec::decode(contents)
}

/// Loads auth config from disk, returning defaults if the file doesn't exist.
pub fn load_from_path(path: &Path) -> Result<AuthConfig> {
    if path.exists() {
        let contents = std::fs::read_to_string(path)?;
        TomlCodec::decode(&contents)
    } else {
        Ok(AuthConfig::default())
    }
}

/// Loads auth config + mtime from the canonical path.
/// Falls back to default on error (logging a warning via eprintln).
fn load_with_mtime() -> (AuthConfig, u64) {
    let config = match load_from_path(Path::new(AUTH_PATH)) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("config: failed to reload {}: {}", AUTH_PATH, e);
            AuthConfig::default()
        }
    };
    let mtime = file_mtime(AUTH_PATH);
    (config, mtime)
}

/// Returns the mtime of a file as seconds since epoch, or 0 if unavailable.
fn file_mtime(path: &str) -> u64 {
    std::fs::metadata(path)
        .and_then(|m| m.modified())
        .map(|t| {
            t.duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs())
                .unwrap_or(0)
        })
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn auth_config_defaults() {
        // ARRANGE & ACT
        let config = AuthConfig::default();

        // ASSERT
        assert!(config.users.is_empty());
        assert!(config.revoked.is_empty());
    }

    #[test]
    fn auth_round_trip() {
        // ARRANGE
        let config = AuthConfig {
            users: vec![AuthUser {
                fingerprint: "abc123".to_string(),
                permissions: vec![Permission::Admin],
            }],
            revoked: vec!["revoked_fp".to_string()],
        };

        // ACT
        let serialized = serialize(&config).unwrap();
        let deserialized = parse(&serialized).unwrap();

        // ASSERT
        assert_eq!(deserialized.users.len(), 1);
        assert_eq!(deserialized.users[0].fingerprint, "abc123");
        assert_eq!(deserialized.revoked, vec!["revoked_fp"]);
    }

    #[test]
    fn auth_multiple_users_and_permissions() {
        // ARRANGE
        let config = AuthConfig {
            users: vec![
                AuthUser {
                    fingerprint: "fp1".to_string(),
                    permissions: vec![Permission::Admin, Permission::VmRead],
                },
                AuthUser {
                    fingerprint: "fp2".to_string(),
                    permissions: vec![Permission::SystemRead],
                },
            ],
            revoked: vec!["dead_fp".to_string(), "another_dead".to_string()],
        };

        // ACT
        let serialized = serialize(&config).unwrap();
        let deserialized = parse(&serialized).unwrap();

        // ASSERT
        assert_eq!(deserialized.users.len(), 2);
        assert_eq!(deserialized.users[0].permissions.len(), 2);
        assert_eq!(deserialized.users[1].fingerprint, "fp2");
        assert_eq!(deserialized.revoked.len(), 2);
    }

    #[test]
    fn serialize_empty_config() {
        // ARRANGE
        let config = AuthConfig::default();

        // ACT
        let serialized = serialize(&config).unwrap();
        let deserialized = parse(&serialized).unwrap();

        // ASSERT
        assert!(deserialized.users.is_empty());
        assert!(deserialized.revoked.is_empty());
    }

    #[test]
    fn load_from_nonexistent_path() {
        // ARRANGE
        let path = Path::new("/nonexistent/auth.toml");

        // ACT
        let config = load_from_path(path).unwrap();

        // ASSERT
        assert!(config.users.is_empty());
        assert!(config.revoked.is_empty());
    }

    #[test]
    fn load_from_tempfile() {
        // ARRANGE
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("auth.toml");
        std::fs::write(
            &path,
            "[[users]]\nfingerprint = \"fp1\"\npermissions = [\"admin\"]\n",
        )
        .unwrap();

        // ACT
        let config = load_from_path(&path).unwrap();

        // ASSERT
        assert_eq!(config.users.len(), 1);
        assert_eq!(config.users[0].fingerprint, "fp1");
    }

    #[test]
    fn load_from_path_invalid_format_returns_error() {
        // ARRANGE
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("auth.toml");
        std::fs::write(&path, "[[[ not valid toml").unwrap();

        // ACT
        let result = load_from_path(&path);

        // ASSERT
        assert!(result.is_err());
    }

    #[test]
    fn load_with_mtime_nonexistent_returns_default() {
        // ACT
        let (config, mtime) = load_with_mtime();

        // ASSERT
        assert!(config.users.is_empty());
        assert_eq!(mtime, 0);
    }

    #[test]
    fn file_mtime_nonexistent_returns_zero() {
        // ACT
        let mtime = file_mtime("/nonexistent/path/to/file.toml");

        // ASSERT
        assert_eq!(mtime, 0);
    }

    #[test]
    fn file_mtime_existing_file_returns_nonzero() {
        // ARRANGE
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.toml");
        std::fs::write(&path, "data").unwrap();

        // ACT
        let mtime = file_mtime(path.to_str().unwrap());

        // ASSERT
        assert!(mtime > 0);
    }

    #[test]
    fn try_auth_returns_none_before_init() {
        // ARRANGE & ACT
        let result = try_auth();

        // ASSERT
        let _ = result;
    }

    #[test]
    fn parse_invalid_format_returns_error() {
        // ARRANGE
        let invalid = "not valid ][[[";

        // ACT
        let result = parse(invalid);

        // ASSERT
        assert!(result.is_err());
    }
}
