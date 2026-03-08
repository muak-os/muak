//! Mutable authentication and authorization state.

use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, OnceLock, RwLock};

use serde::{Deserialize, Serialize};

use crate::Permission;
use crate::error::Result;

pub const AUTH_PATH: &str = "/run/state/auth.toml";
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
    pub users: Vec<AuthUser>,
    pub revoked: Vec<String>,
}

/// An authorized user identified by certificate fingerprint.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthUser {
    pub fingerprint: String,
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

/// Serializes an [`AuthConfig`] to a TOML string.
pub fn serialize(config: &AuthConfig) -> Result<String> {
    toml::to_string_pretty(config).map_err(Into::into)
}

/// Parses an [`AuthConfig`] from a TOML string.
pub fn parse(contents: &str) -> Result<AuthConfig> {
    toml::from_str(contents).map_err(Into::into)
}

/// Loads auth config from disk, returning defaults if the file doesn't exist.
pub fn load_from_path(path: &Path) -> Result<AuthConfig> {
    if path.exists() {
        let contents = std::fs::read_to_string(path)?;
        toml::from_str(&contents).map_err(Into::into)
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
            eprintln!("sysconfig: failed to reload {}: {}", AUTH_PATH, e);
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
    fn test_auth_config_defaults() {
        let config = AuthConfig::default();
        assert!(config.users.is_empty());
        assert!(config.revoked.is_empty());
    }

    #[test]
    fn test_auth_round_trip() {
        let config = AuthConfig {
            users: vec![AuthUser {
                fingerprint: "abc123".to_string(),
                permissions: vec![Permission::Admin],
            }],
            revoked: vec!["revoked_fp".to_string()],
        };

        let serialized = serialize(&config).unwrap();
        let deserialized = parse(&serialized).unwrap();

        assert_eq!(deserialized.users.len(), 1);
        assert_eq!(deserialized.users[0].fingerprint, "abc123");
        assert_eq!(deserialized.revoked, vec!["revoked_fp"]);
    }

    #[test]
    fn test_load_from_nonexistent_path() {
        let config = load_from_path(Path::new("/nonexistent/auth.toml")).unwrap();
        assert!(config.users.is_empty());
        assert!(config.revoked.is_empty());
    }

    #[test]
    fn test_load_from_tempfile() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("auth.toml");
        std::fs::write(
            &path,
            "[[users]]\nfingerprint = \"fp1\"\npermissions = [\"admin\"]\n",
        )
        .unwrap();

        let config = load_from_path(&path).unwrap();
        assert_eq!(config.users.len(), 1);
        assert_eq!(config.users[0].fingerprint, "fp1");
    }

    #[test]
    fn test_file_mtime_nonexistent_returns_zero() {
        let mtime = file_mtime("/nonexistent/path/to/file.toml");
        assert_eq!(mtime, 0);
    }

    #[test]
    fn test_file_mtime_existing_file_returns_nonzero() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.toml");
        std::fs::write(&path, "data").unwrap();
        let mtime = file_mtime(path.to_str().unwrap());
        assert!(mtime > 0);
    }

    #[test]
    fn test_try_auth_returns_none_before_init() {
        let _ = try_auth();
    }

    #[test]
    fn test_parse_invalid_toml_returns_error() {
        let result = parse("not valid toml ][[[");
        assert!(result.is_err());
    }
}
